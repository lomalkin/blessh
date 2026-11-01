//! Start of an L2CAP channel: the client's hello with a probe, the server's
//! reply with the probe echoed, then the SSH connection (docs/protocol.md,
//! "Channel handshake").
//!
//! The probe shows that the channel carries data both ways before SSH
//! starts: some pairs of Bluetooth stacks open a channel and then stall on
//! it (flow control), which a client would otherwise only notice as a hung
//! SSH login. A client that sees the echo stall tries the server's next
//! channel or another transport.

use std::{io, time::Duration};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// First bytes of a hello and of a reply.
pub const MAGIC: [u8; 4] = *b"BLSH";
/// Highest handshake version this implementation speaks.
pub const VERSION: u8 = 1;
/// Largest probe a server echoes.
pub const MAX_PROBE: u16 = 8192;
/// Probe size clients send: about an SSH key exchange init, the first
/// large message on a new connection.
pub const DEFAULT_PROBE: u16 = 2048;
/// Largest options block.
pub const MAX_OPTIONS: u16 = 1024;
/// Header length of a hello and of a reply.
pub const HEADER_LEN: usize = 10;

/// Reply status: the server runs the session.
pub const ACCEPTED: u8 = 0;
/// Reply status: the server refuses the session; the reason option says why.
pub const REFUSED: u8 = 1;

/// Option in a reply: UTF-8 text saying why the session is refused.
pub const OPT_REASON: u8 = 1;

/// Options: type and value, each value at most 255 bytes. Receivers ignore
/// types they do not know.
pub type Options = Vec<(u8, Vec<u8>)>;

fn encode_options(options: &Options) -> Vec<u8> {
    let mut out = Vec::new();
    for (t, v) in options {
        let v = &v[..v.len().min(255)];
        out.push(*t);
        out.push(v.len() as u8);
        out.extend_from_slice(v);
    }
    out
}

fn decode_options(mut b: &[u8]) -> io::Result<Options> {
    let mut out = Vec::new();
    while !b.is_empty() {
        if b.len() < 2 || b.len() < 2 + b[1] as usize {
            return Err(invalid("truncated option"));
        }
        let n = b[1] as usize;
        out.push((b[0], b[2..2 + n].to_vec()));
        b = &b[2 + n..];
    }
    Ok(out)
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn header(version: u8, flags_or_status: u8, probe: u16, options: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + options.len());
    out.extend_from_slice(&MAGIC);
    out.push(version);
    out.push(flags_or_status);
    out.extend_from_slice(&probe.to_le_bytes());
    out.extend_from_slice(&(options.len() as u16).to_le_bytes());
    out.extend_from_slice(options);
    out
}

/// A hello or a reply without the probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    /// Flags in a hello (none defined, ignored), status in a reply.
    pub code: u8,
    pub probe: u16,
    pub options: Options,
}

async fn read_header<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Header> {
    let mut h = [0u8; HEADER_LEN];
    r.read_exact(&mut h).await?;
    if h[..4] != MAGIC {
        return Err(invalid("not a blessh channel (bad magic)"));
    }
    if h[4] == 0 {
        return Err(invalid("version 0"));
    }
    let probe = u16::from_le_bytes([h[6], h[7]]);
    let opt_len = u16::from_le_bytes([h[8], h[9]]);
    if opt_len > MAX_OPTIONS {
        return Err(invalid("options too long"));
    }
    let mut opts = vec![0u8; opt_len as usize];
    r.read_exact(&mut opts).await?;
    Ok(Header {
        version: h[4],
        code: h[5],
        probe,
        options: decode_options(&opts)?,
    })
}

/// Deterministic probe content (xorshift), so a client can check the echo
/// without keeping a copy and nothing on the way compresses it.
pub fn probe_bytes(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9e37_79b9;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// Why a client's handshake failed.
#[derive(Debug)]
pub enum ClientError {
    /// The server refused the session; the channel works.
    Refused(String),
    /// The echo stopped coming: the channel does not carry data reliably.
    Stalled {
        echoed: usize,
        expected: usize,
    },
    /// The server answered with something else than a blessh reply.
    Protocol(String),
    Io(io::Error),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "server refused the session: {r}"),
            Self::Stalled { echoed, expected } => {
                write!(
                    f,
                    "channel stalled ({echoed} of {expected} probe bytes came back)"
                )
            }
            Self::Protocol(e) => write!(f, "unexpected answer: {e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// What a successful handshake found out.
#[derive(Debug, Clone, Copy)]
pub struct Accepted {
    pub version: u8,
    /// Time from sending the hello to the last probe byte coming back.
    pub elapsed: Duration,
}

/// Client side: sends the hello and the probe, reads the reply and the
/// echo. `stall` is how long the echo may stop before the channel counts as
/// stalled.
pub async fn client<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    probe: u16,
    stall: Duration,
) -> Result<Accepted, ClientError> {
    let started = std::time::Instant::now();
    let probe = probe.min(MAX_PROBE);
    let content = probe_bytes(probe as usize);
    let mut hello = header(VERSION, 0, probe, &[]);
    hello.extend_from_slice(&content);
    let (mut r, mut w) = tokio::io::split(stream);

    // Writing and reading at once: the echo starts before the probe is
    // fully sent, and neither side may wait for the other's buffers.
    let send = async {
        w.write_all(&hello).await?;
        w.flush().await
    };
    let receive = async {
        let reply = tokio::time::timeout(stall, read_header(&mut r))
            .await
            .map_err(|_| ClientError::Stalled {
                echoed: 0,
                expected: probe as usize,
            })?
            .map_err(|e| match e.kind() {
                io::ErrorKind::InvalidData => ClientError::Protocol(e.to_string()),
                _ => ClientError::Io(e),
            })?;
        if reply.version > VERSION {
            return Err(ClientError::Protocol(format!(
                "server answered with version {}",
                reply.version
            )));
        }
        if reply.code == REFUSED {
            let reason = reply
                .options
                .iter()
                .find(|(t, _)| *t == OPT_REASON)
                .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
                .unwrap_or_else(|| "no reason given".into());
            return Err(ClientError::Refused(reason));
        }
        if reply.code != ACCEPTED || reply.probe != probe {
            return Err(ClientError::Protocol(format!(
                "status {}, probe {} of {probe}",
                reply.code, reply.probe
            )));
        }
        let mut got = 0usize;
        let mut buf = vec![0u8; 1024];
        while got < probe as usize {
            let want = buf.len().min(probe as usize - got);
            let n = match tokio::time::timeout(stall, r.read(&mut buf[..want])).await {
                Err(_) => {
                    return Err(ClientError::Stalled {
                        echoed: got,
                        expected: probe as usize,
                    })
                }
                Ok(Err(e)) => return Err(ClientError::Io(e)),
                Ok(Ok(0)) => {
                    return Err(ClientError::Io(io::ErrorKind::UnexpectedEof.into()));
                }
                Ok(Ok(n)) => n,
            };
            if buf[..n] != content[got..got + n] {
                return Err(ClientError::Protocol("probe echo differs".into()));
            }
            got += n;
        }
        Ok(reply.version)
    };
    let (sent, received) = tokio::join!(send, receive);
    let version = received?;
    sent.map_err(ClientError::Io)?;
    Ok(Accepted {
        version,
        elapsed: started.elapsed(),
    })
}

/// Server side, first half: reads the client's hello (without the probe).
pub async fn read_hello<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Header> {
    let h = read_header(r).await?;
    if h.probe > MAX_PROBE {
        return Err(invalid("probe too long"));
    }
    Ok(h)
}

/// Server side, second half: refuses the session with `reason`.
pub async fn refuse<W: AsyncWrite + Unpin>(
    w: &mut W,
    hello: &Header,
    reason: &str,
) -> io::Result<()> {
    let opts = encode_options(&vec![(OPT_REASON, reason.as_bytes().to_vec())]);
    w.write_all(&header(hello.version.min(VERSION), REFUSED, 0, &opts))
        .await?;
    w.flush().await
}

/// Server side, second half: accepts the session and echoes the probe as it
/// arrives. The SSH connection follows on the same stream.
pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    hello: &Header,
) -> io::Result<()> {
    s.write_all(&header(
        hello.version.min(VERSION),
        ACCEPTED,
        hello.probe,
        &[],
    ))
    .await?;
    s.flush().await?;
    let mut left = hello.probe as usize;
    let mut buf = vec![0u8; 1024];
    while left > 0 {
        let want = buf.len().min(left);
        let n = s.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        s.write_all(&buf[..n]).await?;
        s.flush().await?;
        left -= n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn serve(
        mut s: tokio::io::DuplexStream,
        refuse_with: Option<&str>,
    ) -> io::Result<Header> {
        let hello = read_hello(&mut s).await?;
        match refuse_with {
            Some(r) => refuse(&mut s, &hello, r).await?,
            None => accept(&mut s, &hello).await?,
        }
        // Then SSH: one line each way.
        if refuse_with.is_none() {
            s.write_all(b"SSH-2.0-test\r\n").await?;
        }
        Ok(hello)
    }

    #[tokio::test]
    async fn accepts_and_echoes() {
        let (mut c, s) = tokio::io::duplex(256);
        let server = tokio::spawn(serve(s, None));
        let ok = client(&mut c, DEFAULT_PROBE, Duration::from_secs(3))
            .await
            .unwrap();
        assert_eq!(ok.version, VERSION);
        let mut banner = [0u8; 14];
        c.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"SSH-2.0-test\r\n");
        let hello = server.await.unwrap().unwrap();
        assert_eq!(hello.probe, DEFAULT_PROBE);
    }

    #[tokio::test]
    async fn reports_refusal() {
        let (mut c, s) = tokio::io::duplex(4096);
        tokio::spawn(serve(s, Some("busy (4 sessions)")));
        match client(&mut c, 64, Duration::from_secs(3)).await {
            Err(ClientError::Refused(r)) => assert_eq!(r, "busy (4 sessions)"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn detects_a_stall() {
        let (mut c, mut s) = tokio::io::duplex(8192);
        tokio::spawn(async move {
            let hello = read_hello(&mut s).await.unwrap();
            s.write_all(&header(1, ACCEPTED, hello.probe, &[]))
                .await
                .unwrap();
            // Echo only part of the probe, then nothing.
            let mut buf = vec![0u8; 100];
            s.read_exact(&mut buf).await.unwrap();
            s.write_all(&buf).await.unwrap();
            std::future::pending::<()>().await;
        });
        match client(&mut c, 1000, Duration::from_secs(3)).await {
            Err(ClientError::Stalled { echoed, expected }) => {
                assert_eq!((echoed, expected), (100, 1000))
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_other_protocols() {
        let (mut c, mut s) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let _ = s.write_all(b"SSH-2.0-OpenSSH_9.9\r\n").await;
            std::future::pending::<()>().await;
        });
        assert!(matches!(
            client(&mut c, 64, Duration::from_secs(3)).await,
            Err(ClientError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn server_answers_with_the_lower_version_and_skips_unknown_options() {
        let (mut c, mut s) = tokio::io::duplex(4096);
        let opts = encode_options(&vec![(200, b"future".to_vec())]);
        let mut hello = header(7, 0, 4, &opts);
        hello.extend_from_slice(b"abcd");
        c.write_all(&hello).await.unwrap();
        let h = read_hello(&mut s).await.unwrap();
        assert_eq!(h.options, vec![(200, b"future".to_vec())]);
        accept(&mut s, &h).await.unwrap();
        let reply = read_header(&mut c).await.unwrap();
        assert_eq!(
            (reply.version, reply.code, reply.probe),
            (VERSION, ACCEPTED, 4)
        );
        let mut echo = [0u8; 4];
        c.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"abcd");
    }

    /// The test vectors of docs/protocol.md, "Channel handshake".
    #[test]
    fn wire_format_matches_the_protocol_test_vectors() {
        let hex = |s: &str| -> Vec<u8> {
            s.split_whitespace()
                .map(|b| u8::from_str_radix(b, 16).unwrap())
                .collect()
        };
        let mut hello = header(1, 0, 4, &[]);
        hello.extend_from_slice(b"abcd");
        assert_eq!(hello, hex("42 4c 53 48 01 00 04 00 00 00 61 62 63 64"));
        let reply = header(1, ACCEPTED, 4, &[]);
        assert_eq!(reply, hex("42 4c 53 48 01 00 04 00 00 00"));
        let opts = encode_options(&vec![(OPT_REASON, b"busy".to_vec())]);
        assert_eq!(
            header(1, REFUSED, 0, &opts),
            hex("42 4c 53 48 01 01 00 00 06 00 01 04 62 75 73 79")
        );
    }

    #[test]
    fn options_round_trip_and_reject_truncation() {
        let o = vec![(1, b"x".to_vec()), (9, vec![])];
        assert_eq!(decode_options(&encode_options(&o)).unwrap(), o);
        assert!(decode_options(&[1, 5, 0]).is_err());
    }
}
