//! Byte-stream relay for the L2CAP transport: moves data between the BLE
//! channel and the session stream (the SSH session on the server, stdin
//! and stdout on the client) until one side ends. The channel carries raw
//! SSH, so there is no framing of our own; what the relay adds is bounded
//! buffering and a clear account of how the session ended.

use std::{collections::VecDeque, future::Future, io};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bytes read from one side and not yet written to the other.
const BUFFER: usize = 16384;

#[derive(Debug)]
pub enum End {
    /// The BLE channel ended: closed by the peer or the link was lost. The
    /// error, if any, says which (the kernel reports both as errors).
    Peer(Option<io::Error>),
    /// The local stream ended (sshd exited, ssh closed its ProxyCommand).
    Local,
    /// The caller's cancel future completed (the server is stopping).
    Cancelled,
}

#[derive(Debug)]
pub struct Outcome {
    pub end: End,
    /// Bytes received from the peer and delivered locally.
    pub from_peer: u64,
    /// Bytes read locally and sent to the peer.
    pub to_peer: u64,
}

/// Relays between `peer` (the BLE channel) and `local` until one of them
/// ends. `is_peer_end` tells errors of the channel that mean "the channel or
/// the link is gone" from faults: the kernel reports a peer's close as an
/// error, not as EOF. Faults are returned as errors.
pub async fn run<P, L, C>(
    peer: P,
    local: L,
    is_peer_end: impl Fn(&io::Error) -> bool,
    cancel: C,
) -> io::Result<Outcome>
where
    P: AsyncRead + AsyncWrite,
    L: AsyncRead + AsyncWrite,
    C: Future<Output = ()>,
{
    let (mut pr, mut pw) = tokio::io::split(peer);
    let (mut lr, mut lw) = tokio::io::split(local);
    tokio::pin!(cancel);
    let mut to_local: VecDeque<u8> = VecDeque::new();
    let mut to_peer: VecDeque<u8> = VecDeque::new();
    let mut pbuf = vec![0u8; 4096];
    let mut lbuf = vec![0u8; 4096];
    let (mut from_peer, mut sent) = (0u64, 0u64);
    let mut local_done = false;
    let outcome = |end, from_peer, to_peer| Outcome {
        end,
        from_peer,
        to_peer,
    };

    loop {
        // After the local side ended, deliver what it wrote, then stop.
        if local_done && to_peer.is_empty() {
            let _ = pw.flush().await;
            let _ = lw.flush().await;
            return Ok(outcome(End::Local, from_peer, sent));
        }
        tokio::select! {
            biased;
            _ = &mut cancel => {
                let _ = lw.flush().await;
                return Ok(outcome(End::Cancelled, from_peer, sent));
            }
            // Single writes keep the branches cancel safe.
            r = pw.write(to_peer.as_slices().0), if !to_peer.is_empty() => {
                match r {
                    Ok(n) if n > 0 => {
                        to_peer.drain(..n);
                        sent += n as u64;
                    }
                    Ok(_) => return Ok(outcome(End::Peer(None), from_peer, sent)),
                    Err(e) if is_peer_end(&e) => return Ok(outcome(End::Peer(Some(e)), from_peer, sent)),
                    Err(e) => return Err(e),
                }
            }
            r = lw.write(to_local.as_slices().0), if !to_local.is_empty() && !local_done => {
                match r {
                    Ok(n) if n > 0 => {
                        to_local.drain(..n);
                        from_peer += n as u64;
                    }
                    _ => local_done = true,
                }
            }
            r = pr.read(&mut pbuf), if to_local.len() < BUFFER => {
                match r {
                    Ok(n) if n > 0 => to_local.extend(&pbuf[..n]),
                    Ok(_) => {
                        let _ = lw.flush().await;
                        return Ok(outcome(End::Peer(None), from_peer, sent));
                    }
                    Err(e) if is_peer_end(&e) => {
                        let _ = lw.flush().await;
                        return Ok(outcome(End::Peer(Some(e)), from_peer, sent));
                    }
                    Err(e) => return Err(e),
                }
            }
            r = lr.read(&mut lbuf), if !local_done && to_peer.len() < BUFFER => {
                match r {
                    Ok(n) if n > 0 => to_peer.extend(&lbuf[..n]),
                    _ => local_done = true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fault(e: &io::Error) -> bool {
        e.raw_os_error() == Some(104)
    }

    #[tokio::test]
    async fn relays_both_ways_until_local_eof() {
        let (peer_a, peer_b) = tokio::io::duplex(64);
        let (local_a, local_b) = tokio::io::duplex(64);
        let relay = tokio::spawn(run(peer_a, local_a, fault, std::future::pending()));
        let (mut pr, mut pw) = tokio::io::split(peer_b);
        let (mut lr, mut lw) = tokio::io::split(local_b);
        let sent: Vec<u8> = (0..50_000u32).map(|i| i as u8).collect();
        let (s1, s2) = (sent.clone(), sent.clone());
        let writer = tokio::spawn(async move {
            pw.write_all(&s1).await.unwrap();
            lw.write_all(&s2).await.unwrap();
            lw.shutdown().await.unwrap();
        });
        let mut got_local = vec![0u8; sent.len()];
        lr.read_exact(&mut got_local).await.unwrap();
        let mut got_peer = vec![0u8; sent.len()];
        pr.read_exact(&mut got_peer).await.unwrap();
        writer.await.unwrap();
        assert_eq!(got_local, sent);
        assert_eq!(got_peer, sent);
        let out = relay.await.unwrap().unwrap();
        assert!(matches!(out.end, End::Local));
        assert_eq!(out.from_peer, sent.len() as u64);
        assert_eq!(out.to_peer, sent.len() as u64);
    }

    #[tokio::test]
    async fn peer_eof_ends_the_relay() {
        let (peer_a, peer_b) = tokio::io::duplex(64);
        let (local_a, _local_b) = tokio::io::duplex(64);
        let relay = tokio::spawn(run(peer_a, local_a, fault, std::future::pending()));
        drop(peer_b);
        let out = relay.await.unwrap().unwrap();
        assert!(matches!(out.end, End::Peer(None)));
    }

    #[tokio::test]
    async fn cancel_ends_the_relay() {
        let (peer_a, _peer_b) = tokio::io::duplex(64);
        let (local_a, _local_b) = tokio::io::duplex(64);
        let out = run(peer_a, local_a, fault, async {}).await.unwrap();
        assert!(matches!(out.end, End::Cancelled));
    }
}
