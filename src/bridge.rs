//! Async glue between a packet transport (BLE characteristic pair) and a
//! byte stream (the SSH session on the server, stdin/stdout on the client).

use std::{collections::VecDeque, future::Future, io, time::Duration};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    link::{Event, Link, LinkError},
    protocol::CloseReason,
};

/// Receives whole packets (one ATT value each). An empty packet means EOF.
#[allow(async_fn_in_trait)]
pub trait PacketRx {
    async fn recv(&mut self) -> io::Result<Vec<u8>>;
}

/// Sends whole packets (one ATT value each).
#[allow(async_fn_in_trait)]
pub trait PacketTx {
    async fn send(&mut self, pkt: &[u8]) -> io::Result<()>;
}

impl PacketRx for bluer::gatt::CharacteristicReader {
    async fn recv(&mut self) -> io::Result<Vec<u8>> {
        bluer::gatt::CharacteristicReader::recv(self).await
    }
}

impl PacketTx for std::sync::Arc<bluer::gatt::CharacteristicWriter> {
    async fn send(&mut self, pkt: &[u8]) -> io::Result<()> {
        bluer::gatt::CharacteristicWriter::send(self, pkt).await
    }
}

/// Upper bound of stream data read ahead of the peer's credit.
const READ_AHEAD: usize = 8192;

#[derive(Debug)]
pub enum Outcome {
    /// The local stream ended (e.g. sshd exited); CLOSE was sent to the peer.
    StreamEnded,
    /// The peer sent CLOSE.
    PeerClosed {
        reason: CloseReason,
        message: String,
    },
    /// The packet transport went away (BLE disconnect).
    LinkLost,
    /// The caller's cancel future completed.
    Cancelled,
}

#[derive(Debug)]
pub enum BridgeError {
    Io(io::Error),
    Protocol(LinkError),
    HandshakeTimeout,
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Protocol(e) => write!(f, "protocol error: {e}"),
            Self::HandshakeTimeout => write!(f, "handshake timed out"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<io::Error> for BridgeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<LinkError> for BridgeError {
    fn from(e: LinkError) -> Self {
        Self::Protocol(e)
    }
}

async fn flush_link<T: PacketTx>(link: &mut Link, tx: &mut T) -> io::Result<()> {
    while let Some(pkt) = link.poll_transmit() {
        log::trace!("tx packet {} bytes", pkt.len());
        tx.send(&pkt).await?;
    }
    Ok(())
}

/// Sends our HELLO and waits for the peer's. Data that arrives together with
/// the peer HELLO is returned so it is not lost.
pub async fn handshake<R: PacketRx, T: PacketTx>(
    link: &mut Link,
    rx: &mut R,
    tx: &mut T,
    timeout: Duration,
) -> Result<Vec<Event>, BridgeError> {
    flush_link(link, tx).await?;
    let wait = async {
        loop {
            let pkt = rx.recv().await?;
            if pkt.is_empty() {
                return Err(BridgeError::Io(io::ErrorKind::UnexpectedEof.into()));
            }
            let events = link.on_packet(&pkt)?;
            if link.is_open() {
                return Ok(events);
            }
        }
    };
    match tokio::time::timeout(timeout, wait).await {
        Ok(r) => r,
        Err(_) => Err(BridgeError::HandshakeTimeout),
    }
}

/// Moves data between `rx`/`tx` and `stream` until one side ends.
///
/// `initial` holds events already produced by [handshake].
pub async fn run<R, T, S, C>(
    link: &mut Link,
    rx: &mut R,
    tx: &mut T,
    stream: S,
    initial: Vec<Event>,
    cancel: C,
) -> Result<Outcome, BridgeError>
where
    R: PacketRx,
    T: PacketTx,
    S: AsyncRead + AsyncWrite,
    C: Future<Output = ()>,
{
    let (mut sr, mut sw) = tokio::io::split(stream);
    tokio::pin!(cancel);
    let mut to_stream: VecDeque<u8> = VecDeque::new();
    let mut buf = vec![0u8; 4096];

    let mut pending = initial;
    let mut stream_done = false;
    loop {
        for ev in pending.drain(..) {
            match ev {
                Event::Data(d) => to_stream.extend(d),
                Event::Closed { reason, message } => {
                    // Deliver what arrived before the CLOSE frame.
                    let (a, b) = to_stream.as_slices();
                    if sw.write_all(a).await.is_ok() && sw.write_all(b).await.is_ok() {
                        let _ = sw.flush().await;
                    }
                    return Ok(Outcome::PeerClosed { reason, message });
                }
                Event::Open { .. } => {}
            }
        }
        flush_link(link, tx).await?;
        if stream_done && link.pending_out() == 0 {
            let _ = sw.flush().await;
            return finish(link, tx).await;
        }

        let can_read = !stream_done && link.is_open() && link.pending_out() < READ_AHEAD;
        tokio::select! {
            biased;
            _ = &mut cancel => {
                let _ = sw.flush().await;
                return Ok(Outcome::Cancelled);
            }
            pkt = rx.recv() => {
                let pkt = match pkt {
                    Ok(p) if !p.is_empty() => p,
                    _ => {
                        let _ = sw.flush().await;
                        return Ok(Outcome::LinkLost);
                    }
                };
                log::trace!("rx packet {} bytes", pkt.len());
                pending = link.on_packet(&pkt)?;
            }
            // A single write keeps this branch cancel safe; a flush in the same
            // future could be dropped after the write was accepted.
            r = sw.write(to_stream.as_slices().0), if !to_stream.is_empty() && !stream_done => {
                match r {
                    Ok(n) if n > 0 => {
                        log::trace!("stream write {n}");
                        to_stream.drain(..n);
                        link.consumed(n);
                    }
                    _ => stream_done = true,
                }
            }
            r = sr.read(&mut buf), if can_read => {
                match r {
                    Ok(n) if n > 0 => {
                        log::trace!("stream read {n}");
                        link.send(&buf[..n]);
                    }
                    // Keep going until the queued output has been delivered.
                    _ => stream_done = true,
                }
            }
        }
    }
}

async fn finish<T: PacketTx>(link: &mut Link, tx: &mut T) -> Result<Outcome, BridgeError> {
    link.close(CloseReason::Normal, "session ended");
    // The peer may already be gone; a failed flush is not an error here.
    let _ = flush_link(link, tx).await;
    Ok(Outcome::StreamEnded)
}

/// stdin/stdout as one stream. stdout is written through an unbuffered file
/// on fd 1: `tokio::io::stdout` goes through std's line buffer, which holds
/// binary data without a trailing newline back indefinitely.
pub fn stdio() -> io::Result<impl AsyncRead + AsyncWrite> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = unsafe { libc::dup(std::io::stdout().as_raw_fd()) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let out = tokio::fs::File::from_std(unsafe { std::fs::File::from_raw_fd(fd) });
    Ok(tokio::io::join(tokio::io::stdin(), out))
}
