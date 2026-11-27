//! Resumable sessions on L2CAP channels (docs/protocol.md, "Resumable
//! sessions"): an SSH session that outlives the channel it started on.
//!
//! A client asks for one with the RESUMABLE option in its channel hello; a
//! server that agrees answers with SESSION (a token and its replay buffer
//! size), and from then on the channel carries records instead of raw SSH:
//! DATA with stream bytes, ACK with how many stream bytes arrived, CLOSE.
//! Each side keeps what it sent until the peer acknowledges it. When the
//! channel or the link goes away, the server keeps the session (and sshd)
//! for a while; the client opens a new channel, sends RESUME with the token
//! and the bytes it received, the server answers with the bytes it
//! received, and each side resends from there. SSH does not notice.
//!
//! Stream offsets count stream bytes from the start of the session, in each
//! direction, independent of channels and links.

use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    future::Future,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::channel::{self, Header, Options};
use crate::relay::MAX_PEER_WRITE;

/// Length of a session token.
pub const TOKEN_LEN: usize = 16;
/// A session token: 128 random bits, new on every resume.
pub type Token = [u8; TOKEN_LEN];

/// Record types on a resumable channel.
pub const REC_DATA: u8 = 1;
pub const REC_ACK: u8 = 2;
pub const REC_CLOSE: u8 = 3;
/// Record header: type u8, value length u16 LE.
pub const RECORD_HEADER: usize = 3;

/// CLOSE reasons (the BleStream CLOSE reasons).
pub const CLOSE_NORMAL: u8 = 0;
pub const CLOSE_PROTOCOL: u8 = 2;
pub const CLOSE_UNAVAILABLE: u8 = 3;
pub const CLOSE_TIMEOUT: u8 = 4;

/// Replay buffer size this implementation announces.
pub const DEFAULT_REPLAY: u32 = 65536;
/// Smallest replay buffer a peer may announce; smaller values are raised
/// to it, so that acknowledgements are never due more than once per byte.
pub const MIN_REPLAY: u32 = 1024;
/// How long received stream bytes may wait for an ACK.
pub const ACK_DELAY: Duration = Duration::from_millis(500);

/// Stream bytes buffered towards the local side (sshd, or the client's SSH
/// library) before the relay stops reading the channel.
const TO_LOCAL_LIMIT: usize = 16384;

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// A new random token from the system's CSPRNG. Fails rather than fall back
/// to something guessable: without a token there is no resumption.
pub fn new_token() -> io::Result<Token> {
    let mut t = [0u8; TOKEN_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact_sync(&mut t)?;
    Ok(t)
}

/// `std::io::Read::read_exact` without importing the trait into the module
/// (it clashes with tokio's).
trait ReadExactSync {
    fn read_exact_sync(&mut self, buf: &mut [u8]) -> io::Result<()>;
}

impl ReadExactSync for std::fs::File {
    fn read_exact_sync(&mut self, buf: &mut [u8]) -> io::Result<()> {
        std::io::Read::read_exact(self, buf)
    }
}

/// The first 4 bytes of a token in hex, for logs: enough to tell sessions
/// apart, not enough to resume one.
pub fn token_tag(t: &Token) -> String {
    t[..4].iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------- options

/// A RESUME option: the session to resume and where to resume it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeRequest {
    pub token: Token,
    /// Stream bytes the client received from the server.
    pub received: u64,
}

/// A SESSION option: the server made the session resumable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub token: Token,
    /// The server's replay buffer size.
    pub replay: u32,
    /// Stream bytes the server received from the client: 0 on a fresh
    /// session, where the client resends from on a resumed one.
    pub received: u64,
}

pub fn resumable_option(replay: u32) -> (u8, Vec<u8>) {
    (channel::OPT_RESUMABLE, replay.to_le_bytes().to_vec())
}

pub fn resume_option(r: &ResumeRequest) -> (u8, Vec<u8>) {
    let mut v = r.token.to_vec();
    v.extend_from_slice(&r.received.to_le_bytes());
    (channel::OPT_RESUME, v)
}

pub fn session_option(g: &Grant) -> (u8, Vec<u8>) {
    let mut v = g.token.to_vec();
    v.extend_from_slice(&g.replay.to_le_bytes());
    v.extend_from_slice(&g.received.to_le_bytes());
    (channel::OPT_SESSION, v)
}

fn u32_at(v: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(v[at..at + 4].try_into().unwrap())
}

fn u64_at(v: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(v[at..at + 8].try_into().unwrap())
}

/// The replay buffer size of a RESUMABLE option, if the hello has one.
/// Receivers ignore bytes after the defined fields, for later extensions.
pub fn parse_resumable(options: &Options) -> Option<u32> {
    let v = channel::option(options, channel::OPT_RESUMABLE)?;
    (v.len() >= 4).then(|| u32_at(v, 0).max(MIN_REPLAY))
}

pub fn parse_resume(options: &Options) -> Option<ResumeRequest> {
    let v = channel::option(options, channel::OPT_RESUME)?;
    (v.len() >= TOKEN_LEN + 8).then(|| ResumeRequest {
        token: v[..TOKEN_LEN].try_into().unwrap(),
        received: u64_at(v, TOKEN_LEN),
    })
}

pub fn parse_session(options: &Options) -> Option<Grant> {
    let v = channel::option(options, channel::OPT_SESSION)?;
    (v.len() >= TOKEN_LEN + 12).then(|| Grant {
        token: v[..TOKEN_LEN].try_into().unwrap(),
        replay: u32_at(v, TOKEN_LEN).max(MIN_REPLAY),
        received: u64_at(v, TOKEN_LEN + 4),
    })
}

// ---------------------------------------------------------------- records

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Data(Vec<u8>),
    Ack(u64),
    Close {
        reason: u8,
        message: String,
    },
    /// A record type this implementation does not know; skipped.
    Unknown(u8),
}

pub fn encode_record(t: u8, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD_HEADER + value.len());
    out.push(t);
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value);
    out
}

pub fn encode_ack(received: u64) -> Vec<u8> {
    encode_record(REC_ACK, &received.to_le_bytes())
}

pub fn encode_close(reason: u8, message: &str) -> Vec<u8> {
    let mut v = vec![reason];
    v.extend_from_slice(&message.as_bytes()[..message.len().min(200)]);
    encode_record(REC_CLOSE, &v)
}

/// Splits the bytes of a channel into records; records may span reads.
#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
}

impl Parser {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete record, if there is one.
    pub fn next_record(&mut self) -> io::Result<Option<Record>> {
        if self.buf.len() < RECORD_HEADER {
            return Ok(None);
        }
        let len = u16::from_le_bytes([self.buf[1], self.buf[2]]) as usize;
        if self.buf.len() < RECORD_HEADER + len {
            return Ok(None);
        }
        let t = self.buf[0];
        let value: Vec<u8> = self
            .buf
            .drain(..RECORD_HEADER + len)
            .skip(RECORD_HEADER)
            .collect();
        Ok(Some(match t {
            REC_DATA => Record::Data(value),
            REC_ACK if value.len() >= 8 => Record::Ack(u64_at(&value, 0)),
            REC_ACK => return Err(invalid("short ACK record")),
            REC_CLOSE if !value.is_empty() => Record::Close {
                reason: value[0],
                message: String::from_utf8_lossy(&value[1..]).into_owned(),
            },
            REC_CLOSE => return Err(invalid("empty CLOSE record")),
            other => Record::Unknown(other),
        }))
    }
}

// ---------------------------------------------------------------- stream state

/// One side's state of a resumable session: what it sent and the peer has
/// not acknowledged yet, and what it received. Lives as long as the
/// session, across channels.
pub struct Stream {
    /// Stream bytes from the local side, from offset `acked` on: sent but
    /// not acknowledged, then not sent yet.
    out: VecDeque<u8>,
    acked: u64,
    /// Offset of the next byte to put on the channel.
    sent: u64,
    cap: usize,
    /// Stream bytes taken from the channel.
    received: u64,
    to_local: VecDeque<u8>,
    /// The last `received` the peer was told (ACK or handshake).
    ack_told: u64,
    peer_replay: u32,
    /// The local side ended (EOF or a failed write); nothing more to read
    /// from it or to write to it.
    local_done: bool,
}

impl Stream {
    /// `cap`: this side's replay buffer size; `peer_replay`: the peer's.
    pub fn new(cap: u32, peer_replay: u32) -> Self {
        Self {
            out: VecDeque::new(),
            acked: 0,
            sent: 0,
            cap: cap.max(MIN_REPLAY) as usize,
            received: 0,
            to_local: VecDeque::new(),
            ack_told: 0,
            peer_replay: peer_replay.max(MIN_REPLAY),
            local_done: false,
        }
    }

    pub fn received(&self) -> u64 {
        self.received
    }

    /// Stream bytes read from the local side so far.
    pub fn read_from_local(&self) -> u64 {
        self.acked + self.out.len() as u64
    }

    pub fn local_done(&self) -> bool {
        self.local_done
    }

    /// Prepares a new channel: the peer received `peer_received` bytes, so
    /// sending goes on from there, and the peer knows our `received` from
    /// the handshake. Fails if those bytes are no longer held (the peer
    /// claims less than it acknowledged) or were never sent.
    pub fn rewind(&mut self, peer_received: u64) -> Result<(), &'static str> {
        if peer_received < self.acked {
            return Err("offset already acknowledged");
        }
        if peer_received > self.read_from_local() {
            return Err("offset beyond the data sent");
        }
        self.release(peer_received);
        self.sent = peer_received;
        self.ack_told = self.received;
        Ok(())
    }

    /// Sets the peer's replay buffer size from a resumed channel's handshake.
    pub fn set_peer_replay(&mut self, replay: u32) {
        self.peer_replay = replay.max(MIN_REPLAY);
    }

    fn release(&mut self, upto: u64) {
        let n = (upto - self.acked) as usize;
        self.out.drain(..n);
        self.acked = upto;
    }

    fn ack(&mut self, upto: u64) -> io::Result<()> {
        if upto > self.sent {
            return Err(invalid("ACK for bytes not sent"));
        }
        if upto > self.acked {
            self.release(upto);
        }
        Ok(())
    }

    fn room(&self) -> usize {
        self.cap.saturating_sub(self.out.len())
    }

    fn unsent(&self) -> usize {
        (self.read_from_local() - self.sent) as usize
    }

    /// The next DATA record, at most `max` bytes long with its header.
    fn take_data(&mut self, max: usize) -> Option<Vec<u8>> {
        let n = self.unsent().min(max.saturating_sub(RECORD_HEADER));
        if n == 0 {
            return None;
        }
        let from = (self.sent - self.acked) as usize;
        let chunk: Vec<u8> = self.out.range(from..from + n).copied().collect();
        self.sent += n as u64;
        Some(encode_record(REC_DATA, &chunk))
    }

    fn all_acked(&self) -> bool {
        self.out.is_empty()
    }
}

// ---------------------------------------------------------------- relay

/// How one channel's part of a session ended.
#[derive(Debug)]
pub enum Ended<T> {
    /// The channel ended or failed: the session may resume on another.
    Channel(Option<io::Error>),
    /// The peer ended the session with CLOSE.
    Closed { reason: u8, message: String },
    /// The local side ended, everything it wrote was acknowledged, and
    /// CLOSE was sent.
    Local,
    /// A record the protocol does not allow; CLOSE 2 was sent.
    Protocol(io::Error),
    /// The `interrupt` future completed with this value.
    Interrupted(T),
}

/// Pumps a session's bytes between `channel` and `local` until the channel
/// ends, the session ends, or `interrupt` completes. Bytes in flight stay
/// in `state`, so a later call on a new channel continues where this one
/// stopped.
pub async fn run_attached<C, L, I, T>(
    channel: C,
    local: &mut L,
    state: &mut Stream,
    interrupt: I,
) -> Ended<T>
where
    C: AsyncRead + AsyncWrite,
    L: AsyncRead + AsyncWrite + Unpin,
    I: Future<Output = T>,
{
    let (mut cr, mut cw) = tokio::io::split(channel);
    let (mut lr, mut lw) = tokio::io::split(local);
    tokio::pin!(interrupt);
    let mut parser = Parser::default();
    let mut wbuf: VecDeque<u8> = VecDeque::new();
    let mut cbuf = vec![0u8; 4096];
    let mut lbuf = vec![0u8; 4096];
    let mut ack_at: Option<tokio::time::Instant> = None;

    loop {
        // Queue what is due: an ACK, then DATA up to one write's worth.
        let unacked = state.received - state.ack_told;
        if unacked > 0
            && (unacked >= u64::from(state.peer_replay / 4)
                || ack_at.is_some_and(|t| t <= tokio::time::Instant::now()))
        {
            wbuf.extend(encode_ack(state.received));
            state.ack_told = state.received;
            ack_at = None;
        } else if unacked > 0 && ack_at.is_none() {
            ack_at = Some(tokio::time::Instant::now() + ACK_DELAY);
        }
        while wbuf.len() < MAX_PEER_WRITE {
            match state.take_data(MAX_PEER_WRITE - wbuf.len()) {
                Some(r) => wbuf.extend(r),
                None => break,
            }
        }
        if state.local_done && state.all_acked() && state.unsent() == 0 && wbuf.is_empty() {
            // A last ACK, so the peer knows everything arrived, then CLOSE.
            let mut close = Vec::new();
            if state.received > state.ack_told {
                close.extend(encode_ack(state.received));
                state.ack_told = state.received;
            }
            close.extend(encode_close(CLOSE_NORMAL, ""));
            let _ = cw.write_all(&close).await;
            let _ = cw.flush().await;
            return Ended::Local;
        }
        let ack_timer = async {
            match ack_at {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            v = &mut interrupt => return Ended::Interrupted(v),
            r = cw.write(head(&wbuf, MAX_PEER_WRITE)), if !wbuf.is_empty() => match r {
                Ok(n) if n > 0 => { wbuf.drain(..n); }
                Ok(_) => return Ended::Channel(None),
                Err(e) => return Ended::Channel(Some(e)),
            },
            r = lw.write(state.to_local.as_slices().0), if !state.to_local.is_empty() && !state.local_done => match r {
                Ok(n) if n > 0 => { state.to_local.drain(..n); }
                _ => { state.local_done = true; state.to_local.clear(); }
            },
            r = cr.read(&mut cbuf), if state.to_local.len() < TO_LOCAL_LIMIT => match r {
                Ok(0) => return Ended::Channel(None),
                Err(e) => return Ended::Channel(Some(e)),
                Ok(n) => {
                    parser.push(&cbuf[..n]);
                    loop {
                        match parser.next_record() {
                            Ok(None) => break,
                            Ok(Some(Record::Data(d))) => {
                                state.received += d.len() as u64;
                                if !state.local_done {
                                    state.to_local.extend(d);
                                }
                            }
                            Ok(Some(Record::Ack(a))) => {
                                if let Err(e) = state.ack(a) {
                                    return protocol_error(&mut cw, e).await;
                                }
                            }
                            Ok(Some(Record::Close { reason, message })) => {
                                // Deliver what arrived before CLOSE.
                                if !state.local_done {
                                    let (a, b) = state.to_local.as_slices();
                                    let _ = lw.write_all(a).await;
                                    let _ = lw.write_all(b).await;
                                    let _ = lw.flush().await;
                                    state.to_local.clear();
                                }
                                return Ended::Closed { reason, message };
                            }
                            Ok(Some(Record::Unknown(_))) => {}
                            Err(e) => return protocol_error(&mut cw, e).await,
                        }
                    }
                }
            },
            r = lr.read(&mut lbuf[..state.room().min(4096)]), if !state.local_done && state.room() > 0 => match r {
                Ok(n) if n > 0 => state.out.extend(&lbuf[..n]),
                _ => state.local_done = true,
            },
            _ = ack_timer => {}
        }
    }
}

async fn protocol_error<W: AsyncWrite + Unpin, T>(w: &mut W, e: io::Error) -> Ended<T> {
    let _ = w
        .write_all(&encode_close(CLOSE_PROTOCOL, &e.to_string()))
        .await;
    let _ = w.flush().await;
    Ended::Protocol(e)
}

/// The front of `buf`, at most `max` bytes, as one contiguous slice.
fn head(buf: &VecDeque<u8>, max: usize) -> &[u8] {
    let s = buf.as_slices().0;
    &s[..s.len().min(max)]
}

/// Keeps a detached session's local side moving while no channel is
/// attached: delivers what arrived, and reads the local side until the
/// replay buffer is full. Returns when `wake` completes.
pub async fn run_detached<L, W, T>(local: &mut L, state: &mut Stream, wake: W) -> T
where
    L: AsyncRead + AsyncWrite + Unpin,
    W: Future<Output = T>,
{
    let (mut lr, mut lw) = tokio::io::split(local);
    tokio::pin!(wake);
    let mut lbuf = vec![0u8; 4096];
    loop {
        tokio::select! {
            biased;
            v = &mut wake => return v,
            r = lw.write(state.to_local.as_slices().0), if !state.to_local.is_empty() && !state.local_done => match r {
                Ok(n) if n > 0 => { state.to_local.drain(..n); }
                _ => { state.local_done = true; state.to_local.clear(); }
            },
            r = lr.read(&mut lbuf[..state.room().min(4096)]), if !state.local_done && state.room() > 0 => match r {
                Ok(n) if n > 0 => state.out.extend(&lbuf[..n]),
                _ => state.local_done = true,
            },
        }
    }
}

// ---------------------------------------------------------------- server

/// A channel a session can run on.
pub trait Channel: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Channel for T {}

/// A channel handed to a session after its hello was read.
pub struct Attach {
    pub channel: Box<dyn Channel>,
    pub hello: Header,
    /// Kept alive as long as the channel is: the caller's per-channel
    /// bookkeeping (the Linux server counts channels per link).
    pub keep: Box<dyn Any + Send>,
    /// Who is attaching, for logs.
    pub peer: String,
}

/// Sessions that can be resumed, by token.
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<Mutex<HashMap<Token, mpsc::UnboundedSender<Attach>>>>,
}

impl Registry {
    /// Hands `attach` to the session with `token`; gives it back if there
    /// is no such session.
    pub fn route(&self, token: &Token, attach: Attach) -> Result<(), Attach> {
        let tx = self.inner.lock().unwrap().get(token).cloned();
        match tx {
            Some(tx) => tx.send(attach).map_err(|e| e.0),
            None => Err(attach),
        }
    }

    /// Sessions in the registry.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn insert(&self, t: Token, tx: mpsc::UnboundedSender<Attach>) {
        self.inner.lock().unwrap().insert(t, tx);
    }

    fn remove(&self, t: &Token) {
        self.inner.lock().unwrap().remove(t);
    }
}

/// Settings of a server's resumable sessions.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// How long a session without a channel is kept.
    pub linger: Duration,
    /// This side's replay buffer size.
    pub replay: u32,
}

/// How a resumable session ended, for the server's log.
#[derive(Debug)]
pub struct Summary {
    pub end: String,
    pub from_client: u64,
    pub to_client: u64,
    /// Channels the session ran on after the first.
    pub resumes: u32,
}

/// Refuses a resume on a channel whose hello asked for one: unknown or
/// expired session.
pub async fn refuse_resume(mut a: Attach, reason: &str) {
    let _ = channel::refuse_with(
        &mut a.channel,
        &a.hello,
        reason,
        vec![(channel::OPT_RESUME_FAILED, Vec::new())],
    )
    .await;
    let _ = a.channel.shutdown().await;
}

/// Runs a resumable session on the server: answers `first` (a hello with
/// RESUMABLE and no RESUME), then pumps between the channel and `local`
/// (the session socket to sshd), keeps the session across lost channels
/// for `config.linger`, and takes channels that resume it from `registry`.
/// `log` receives one line per event.
pub async fn serve<L, S>(
    mut local: L,
    first: Attach,
    registry: Registry,
    config: Config,
    shutdown: S,
    log: impl Fn(&str),
) -> Summary
where
    L: AsyncRead + AsyncWrite + Unpin,
    S: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    let (tx, mut rx) = mpsc::unbounded_channel::<Attach>();
    let mut token = match new_token() {
        Ok(t) => t,
        Err(e) => {
            return Summary {
                end: format!("no token: {e}"),
                from_client: 0,
                to_client: 0,
                resumes: 0,
            }
        }
    };
    registry.insert(token, tx.clone());
    let peer_replay = parse_resumable(&first.hello.options).unwrap_or(MIN_REPLAY);
    let mut state = Stream::new(config.replay, peer_replay);
    let mut next = Some(first);
    let mut attached_once = false;
    let mut resumes = 0u32;
    enum Wake {
        Attach(Attach),
        Shutdown,
        Expired,
    }

    let end = 'session: loop {
        // Attach a channel: the first one, a resume, or a takeover.
        if let Some(mut a) = next.take() {
            if attached_once {
                let Some(req) = parse_resume(&a.hello.options) else {
                    refuse_resume(a, "not a resume").await;
                    continue;
                };
                if req.token != token {
                    refuse_resume(a, "unknown session").await;
                    continue;
                }
                if let Err(why) = state.rewind(req.received) {
                    log(&format!("{}: cannot resume: {why}", a.peer));
                    refuse_resume(a, why).await;
                    continue;
                }
                if let Some(r) = parse_resumable(&a.hello.options) {
                    state.set_peer_replay(r);
                }
                let fresh = match new_token() {
                    Ok(t) => t,
                    Err(_) => token,
                };
                registry.remove(&token);
                token = fresh;
                registry.insert(token, tx.clone());
                resumes += 1;
                log(&format!(
                    "{}: resumed (resume {resumes}), resending from {} of {}, {} received",
                    a.peer,
                    req.received,
                    state.read_from_local(),
                    state.received()
                ));
            }
            let grant = Grant {
                token,
                replay: config.replay,
                received: state.received(),
            };
            if let Err(e) =
                channel::accept_with(&mut a.channel, &a.hello, &vec![session_option(&grant)]).await
            {
                log(&format!("{}: channel handshake failed: {e}", a.peer));
                attached_once = true;
            } else {
                attached_once = true;
                let keep = a.keep;
                let ended = run_attached(a.channel, &mut local, &mut state, async {
                    tokio::select! {
                        biased;
                        _ = &mut shutdown => Wake::Shutdown,
                        a = rx.recv() => match a {
                            Some(a) => Wake::Attach(a),
                            None => Wake::Shutdown,
                        },
                    }
                })
                .await;
                drop(keep);
                match ended {
                    Ended::Interrupted(Wake::Attach(a)) => {
                        log(&format!(
                            "{}: takes the session over from the previous channel",
                            a.peer
                        ));
                        next = Some(a);
                        continue;
                    }
                    Ended::Interrupted(Wake::Shutdown | Wake::Expired) => {
                        break 'session "server is shutting down".to_owned()
                    }
                    Ended::Local => break 'session "server ended the session".to_owned(),
                    Ended::Closed { reason, message } => {
                        break 'session format!(
                            "client closed the session (reason {reason}{}{message})",
                            if message.is_empty() { "" } else { ": " }
                        )
                    }
                    Ended::Protocol(e) => break 'session format!("protocol error: {e}"),
                    Ended::Channel(e) => {
                        let why = e.map_or_else(|| "channel closed".to_owned(), |e| e.to_string());
                        log(&format!(
                            "channel lost ({why}); keeping the session for {}s",
                            config.linger.as_secs()
                        ));
                    }
                }
            }
        }
        // Detached: wait for a resume, keep the local side moving.
        let wake = run_detached(&mut local, &mut state, async {
            tokio::select! {
                biased;
                _ = &mut shutdown => Wake::Shutdown,
                a = rx.recv() => a.map_or(Wake::Shutdown, Wake::Attach),
                _ = tokio::time::sleep(config.linger) => Wake::Expired,
            }
        })
        .await;
        match wake {
            Wake::Attach(a) => next = Some(a),
            Wake::Shutdown => break 'session "server is shutting down".to_owned(),
            Wake::Expired => {
                break 'session format!("not resumed within {}s", config.linger.as_secs())
            }
        }
    };
    registry.remove(&token);
    Summary {
        end,
        from_client: state.received(),
        to_client: state.acked,
        resumes,
    }
}

// ---------------------------------------------------------------- client

/// The options of a client's channel hello: RESUMABLE, and RESUME when the
/// channel continues a session.
pub fn client_options(replay: u32, resume: Option<&ResumeRequest>) -> Options {
    let mut o = vec![resumable_option(replay)];
    if let Some(r) = resume {
        o.push(resume_option(r));
    }
    o
}

/// What a client's attempt to open a channel for a session brought.
pub enum Reconnect<C> {
    /// A channel whose handshake answered with SESSION.
    Attached(C, Grant),
    /// The server refused the resume: the session is gone.
    CannotResume(String),
    /// No channel this time (no link, a stalled channel); try again.
    Failed(String),
}

/// How a client's resumable session ended.
#[derive(Debug)]
pub enum ClientEnd {
    /// The local side (ssh) ended and everything it sent arrived.
    Local,
    /// The server ended the session with CLOSE.
    Closed { reason: u8, message: String },
    /// A record the protocol does not allow.
    Protocol(io::Error),
    /// `stop` completed: the client is exiting.
    Cancelled,
    /// The channel was lost and the session could not be resumed.
    Lost(String),
}

/// How a client's resumable session went, for its log and exit code.
#[derive(Debug)]
pub struct ClientSummary {
    pub end: ClientEnd,
    pub from_server: u64,
    pub to_server: u64,
    /// Channels the session ran on after the first.
    pub resumes: u32,
}

/// Settings of a client's resumable session.
#[derive(Clone, Copy, Debug)]
pub struct ClientConfig {
    /// This side's replay buffer size.
    pub replay: u32,
    /// How long after losing the channel the client keeps trying to resume.
    pub give_up: Duration,
}

/// How long a client that is exiting waits for the server to close the
/// channel after its CLOSE.
pub const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// Wait between two failed attempts to open a channel for a resume.
pub const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Runs the client side of a resumable session: pumps between `first` (a
/// channel whose handshake granted `grant`) and `local` (ssh's stdio).
/// When the channel is lost, it keeps `local` open and calls `reconnect`
/// with the resume request until a channel attaches, the server refuses
/// the resume, or `config.give_up` has passed since the loss. `stop` ends the
/// session with CLOSE. `log` receives one line per event.
pub async fn run_client<C, L, R, F, S>(
    local: &mut L,
    first: C,
    grant: Grant,
    config: ClientConfig,
    mut reconnect: R,
    stop: S,
    log: impl Fn(&str),
) -> ClientSummary
where
    C: AsyncRead + AsyncWrite + Unpin,
    L: AsyncRead + AsyncWrite + Unpin,
    R: FnMut(ResumeRequest) -> F,
    F: Future<Output = Reconnect<C>>,
    S: Future<Output = ()>,
{
    tokio::pin!(stop);
    let ClientConfig { replay, give_up } = config;
    let mut state = Stream::new(replay, grant.replay);
    let mut token = grant.token;
    let mut chan = first;
    let mut resumes = 0u32;
    let end = 'session: loop {
        match run_attached(&mut chan, local, &mut state, &mut stop).await {
            Ended::Local => break ClientEnd::Local,
            Ended::Closed { reason, message } => break ClientEnd::Closed { reason, message },
            Ended::Protocol(e) => break ClientEnd::Protocol(e),
            Ended::Interrupted(()) => {
                // ssh hangs up on its proxy as it exits: CLOSE has to reach
                // the server before this process goes, or the server keeps
                // the session for its whole linger time. The server closes
                // the channel once CLOSE arrives.
                let _ = chan
                    .write_all(&encode_close(CLOSE_NORMAL, "client exiting"))
                    .await;
                let _ = chan.flush().await;
                let _ = tokio::time::timeout(CLOSE_WAIT, async {
                    let mut buf = [0u8; 512];
                    while matches!(chan.read(&mut buf).await, Ok(n) if n > 0) {}
                })
                .await;
                let _ = chan.shutdown().await;
                break ClientEnd::Cancelled;
            }
            Ended::Channel(e) => {
                let why = e.map_or_else(|| "channel closed".to_owned(), |e| e.to_string());
                log(&format!(
                    "channel lost ({why}); resuming the session for up to {}s",
                    give_up.as_secs()
                ));
            }
        }
        let deadline = tokio::time::Instant::now() + give_up;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let req = ResumeRequest {
                token,
                received: state.received(),
            };
            enum Wake<C> {
                Stop,
                Expired,
                Tried(Reconnect<C>),
            }
            let tried = reconnect(req);
            let wake = run_detached(local, &mut state, async {
                tokio::select! {
                    biased;
                    _ = &mut stop => Wake::Stop,
                    _ = tokio::time::sleep_until(deadline) => Wake::Expired,
                    r = tried => Wake::Tried(r),
                }
            })
            .await;
            match wake {
                Wake::Stop => break 'session ClientEnd::Cancelled,
                Wake::Expired => {
                    break 'session ClientEnd::Lost(format!(
                        "not resumed within {}s",
                        give_up.as_secs()
                    ))
                }
                Wake::Tried(Reconnect::CannotResume(why)) => {
                    break 'session ClientEnd::Lost(format!("server cannot resume: {why}"))
                }
                Wake::Tried(Reconnect::Failed(why)) => {
                    log(&format!("resume attempt {attempt}: {why}"));
                    let wait = run_detached(local, &mut state, async {
                        tokio::select! {
                            biased;
                            _ = &mut stop => Some(ClientEnd::Cancelled),
                            _ = tokio::time::sleep_until(deadline) => None,
                            _ = tokio::time::sleep(RETRY_DELAY) => None,
                        }
                    })
                    .await;
                    if let Some(end) = wait {
                        break 'session end;
                    }
                }
                Wake::Tried(Reconnect::Attached(mut c, g)) => {
                    if let Err(why) = state.rewind(g.received) {
                        let _ = c.write_all(&encode_close(CLOSE_PROTOCOL, why)).await;
                        let _ = c.flush().await;
                        break 'session ClientEnd::Lost(format!("cannot resume: {why}"));
                    }
                    state.set_peer_replay(g.replay);
                    token = g.token;
                    chan = c;
                    resumes += 1;
                    log(&format!(
                        "resumed (resume {resumes}, attempt {attempt}), resending from {} of {}, {} received",
                        g.received,
                        state.read_from_local(),
                        state.received()
                    ));
                    break;
                }
            }
        }
    };
    ClientSummary {
        end,
        from_server: state.received(),
        to_server: state.acked,
        resumes,
    }
}

/// A channel that fails once `lost` fires: for platforms that report a
/// lost link apart from the channel's streams (CoreBluetooth).
pub struct Watched<S> {
    inner: S,
    /// None once the sender fired or went away.
    lost: Option<tokio::sync::oneshot::Receiver<()>>,
    gone: bool,
}

impl<S> Watched<S> {
    pub fn new(inner: S, lost: tokio::sync::oneshot::Receiver<()>) -> Self {
        Self {
            inner,
            lost: Some(lost),
            gone: false,
        }
    }

    fn check(&mut self, cx: &mut std::task::Context<'_>) -> bool {
        if let Some(rx) = self.lost.as_mut() {
            // A dropped sender means nobody watches: not a loss.
            if let std::task::Poll::Ready(r) = std::pin::Pin::new(rx).poll(cx) {
                self.gone = r.is_ok();
                self.lost = None;
            }
        }
        self.gone
    }
}

fn link_lost() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "link lost")
}

impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        if self.check(cx) {
            return std::task::Poll::Ready(Err(link_lost()));
        }
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        if self.check(cx) {
            return std::task::Poll::Ready(Err(link_lost()));
        }
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
