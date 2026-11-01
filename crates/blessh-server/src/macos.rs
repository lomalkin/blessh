//! `blessh server run` on macOS: CoreBluetooth through blessh-ble. Runs in
//! the user's session (macOS grants Bluetooth to logged-in users, not to
//! daemons), started by `brew services`. Every L2CAP channel becomes one
//! TCP connection to the localhost sshd job that `blessh server enable`
//! installs (docs/macos-server.md), or one run of a command for tests.

use std::{
    error::Error,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use blessh_ble::corebluetooth::{
    self,
    peripheral::{Peripheral, Session},
};
use blessh_core::{
    channel, logging, name,
    relay::{self, End},
    session,
};
use clap::Parser;
use log::{error, info, warn};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, oneshot, watch},
};

#[derive(Parser, Debug, Clone)]
#[command(version, about = "SSH over Bluetooth LE: CoreBluetooth server")]
pub(crate) struct Args {
    /// Advertised name, by host name rules (default: from --name-file,
    /// else the computer's HostName or ComputerName, read at every start;
    /// the " (2)" macOS appends to a ComputerName on a network conflict is
    /// dropped, so such renames do not change it).
    #[arg(long)]
    name: Option<String>,
    /// Optional file with the advertised name, to use another one than the
    /// computer's.
    #[arg(long, default_value = "/etc/blessh/name")]
    name_file: PathBuf,
    /// Print the name the server would advertise and exit.
    #[arg(long)]
    print_name: bool,
    /// Port of the sshd job on 127.0.0.1 (`blessh server enable`).
    #[arg(long, default_value_t = SSHD_PORT)]
    sshd_port: u16,
    /// Run this command for every session instead of using the socket; its
    /// stdin and stdout carry the session (for tests: `sshd -i -f config`).
    #[arg(long)]
    session_command: Option<String>,
    /// Most sessions at a time.
    #[arg(long, default_value_t = 4)]
    max_sessions: usize,
    /// SSH host public key whose fingerprint is logged at start-up.
    #[arg(long, default_value = "/etc/ssh/ssh_host_ed25519_key.pub")]
    host_key: PathBuf,
}

/// How long a client has for its channel hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long Bluetooth may take to come up and the service to register.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// Port of the localhost sshd job.
pub(crate) const SSHD_PORT: u16 = 22022;
/// The name file read when no --name is given.
pub(crate) const NAME_FILE: &str = "/etc/blessh/name";
/// How often the server looks whether its program was replaced.
const UPDATE_CHECK: Duration = Duration::from_secs(10);

pub fn main(argv: Vec<String>) -> i32 {
    logging::init("info");
    let args = Args::parse_from(argv);
    let name = match server_name(args.name.as_deref(), &args.name_file) {
        Ok(n) => n,
        Err(e) => {
            error!("{e}");
            return 2;
        }
    };
    if args.print_name {
        println!("{name}");
        return 0;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match rt.block_on(serve(args, name)) {
        Ok(()) => 0,
        Err(e) => {
            error!("{e}");
            1
        }
    };
    rt.shutdown_background();
    code
}

/// The name to advertise: `--name`, else the name file, else one derived
/// from the computer's names.
pub(crate) fn server_name(given: Option<&str>, name_file: &Path) -> Result<String, String> {
    if let Some(n) = given {
        name::check(n).map_err(|e| format!("--name {n:?}: {e}"))?;
        return Ok(n.to_owned());
    }
    match std::fs::read_to_string(name_file) {
        Ok(s) => {
            let n = s.trim();
            name::check(n).map_err(|e| format!("{}: {n:?}: {e}", name_file.display()))?;
            return Ok(n.to_owned());
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", name_file.display())),
    }
    ["HostName", "ComputerName", "LocalHostName"]
        .iter()
        .filter_map(|k| scutil_get(k))
        .chain(std::iter::once(host_name()))
        .find_map(|n| name::derive(&n))
        .ok_or_else(|| "no name to advertise; set one with --name".into())
}

fn scutil_get(key: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", key])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return "blessh".into();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).into_owned();
    // Short name, like Linux's hostname.
    name.split('.').next().unwrap_or("blessh").to_owned()
}

async fn serve(args: Args, name: String) -> Result<(), Box<dyn Error>> {
    match std::fs::read_to_string(&args.host_key) {
        Ok(k) => info!("SSH host key: {}", k.trim()),
        Err(e) => warn!("cannot read host key {}: {e}", args.host_key.display()),
    }
    if args.session_command.is_none() && !Path::new(crate::macos_admin::DAEMON_PLIST).exists() {
        warn!(
            "the sshd job is not installed ({}): sessions are refused until \
             `blessh server enable` installs it",
            crate::macos_admin::DAEMON_PLIST
        );
    }
    let program = Program::now();
    let (sessions_tx, mut sessions_rx) = mpsc::channel::<io::Result<Session>>(8);
    let (ready_tx, ready_rx) = oneshot::channel::<io::Result<u16>>();
    let advertised = name.clone();
    // CoreBluetooth calls block; the peripheral lives on its own thread.
    std::thread::Builder::new()
        .name("blessh-ble".into())
        .spawn(move || {
            let peripheral = match Peripheral::start(&advertised, START_TIMEOUT) {
                Ok(p) => p,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(peripheral.psm));
            loop {
                let next = peripheral.next_session();
                let fatal = next.is_err();
                if sessions_tx.blocking_send(next).is_err() || fatal {
                    break;
                }
            }
        })?;
    let psm = ready_rx.await??;
    info!("ready (advertising as {name:?}, L2CAP channel on PSM 0x{psm:02x})");

    let count = Arc::new(AtomicUsize::new(0));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut update_check = tokio::time::interval(UPDATE_CHECK);
    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            _ = update_check.tick() => {
                // After an upgrade the service runs the old program until it
                // restarts; restart (launchd's KeepAlive) once idle.
                if count.load(Ordering::SeqCst) == 0 && program.as_ref().is_some_and(Program::replaced) {
                    info!("the program was updated; restarting");
                    break;
                }
            }
            next = sessions_rx.recv() => match next {
                Some(Ok(session)) => {
                    tokio::spawn(run_session(session, args.clone(), count.clone(), shutdown_rx.clone()));
                }
                Some(Err(e)) => return Err(format!("Bluetooth: {e}").into()),
                None => return Err("the Bluetooth thread ended".into()),
            },
        }
    }
    info!("shutting down");
    let _ = shutdown_tx.send(true);
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(())
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

type Local = Box<dyn Stream>;

/// The program this server runs from, to notice an upgrade: the path it was
/// started as (Homebrew's `opt/blessh/bin/blessh`, a link into the current
/// version) and the file that path resolves to.
struct Program {
    path: PathBuf,
    file: (u64, u64),
}

impl Program {
    fn now() -> Option<Self> {
        let path = PathBuf::from(std::env::args_os().next()?);
        let file = Self::identity(&path)?;
        Some(Self { path, file })
    }

    fn identity(path: &Path) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path).ok()?;
        Some((m.dev(), m.ino()))
    }

    fn replaced(&self) -> bool {
        Self::identity(&self.path).is_some_and(|f| f != self.file)
    }
}

/// The session's local end: a TCP connection to the localhost sshd job,
/// or a command's stdio. The child, if any, is returned so that it lives
/// as long as the session.
async fn local_end(args: &Args, tag: &str) -> io::Result<(Local, Option<tokio::process::Child>)> {
    if let Some(cmd) = &args.session_command {
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdio = tokio::io::join(child.stdout.take().unwrap(), child.stdin.take().unwrap());
        return Ok((Box::new(stdio), Some(child)));
    }
    let s = TcpStream::connect(("127.0.0.1", args.sshd_port)).await?;
    // sshd logs this port: it joins the session to sshd's lines.
    if let Ok(a) = s.local_addr() {
        info!("{tag}: to sshd from 127.0.0.1 port {}", a.port());
    }
    Ok((Box::new(s), None))
}

async fn run_session(
    session: Session,
    args: Args,
    count: Arc<AtomicUsize>,
    mut shutdown: watch::Receiver<bool>,
) {
    let Session {
        mut stream,
        central,
    } = session;
    let id = session::new_id();
    let started = Instant::now();
    info!("{central}: session {id}: client connected (l2cap)");
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, channel::read_hello(&mut stream)).await {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            info!("{central}: session {id}: no blessh hello: {e}");
            return;
        }
        Err(_) => {
            info!("{central}: session {id}: no hello in time");
            return;
        }
    };
    if count.fetch_add(1, Ordering::SeqCst) >= args.max_sessions {
        count.fetch_sub(1, Ordering::SeqCst);
        info!(
            "{central}: session {id}: refused, {} sessions in progress",
            args.max_sessions
        );
        let _ = channel::refuse(
            &mut stream,
            &hello,
            &format!("busy ({} sessions)", args.max_sessions),
        )
        .await;
        let _ = stream.shutdown().await;
        return;
    }
    let (local, _child) = match local_end(&args, &format!("{central}: session {id}")).await {
        Ok(l) => l,
        Err(e) => {
            let reason = if e.kind() == io::ErrorKind::ConnectionRefused {
                "SSH is not enabled on this Mac (blessh server enable)"
            } else {
                "session unavailable"
            };
            error!(
                "{central}: session {id}: cannot reach sshd on 127.0.0.1 port {}: {e}",
                args.sshd_port
            );
            let _ = channel::refuse(&mut stream, &hello, reason).await;
            let _ = stream.shutdown().await;
            count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
    };
    if let Err(e) = channel::accept(&mut stream, &hello).await {
        info!("{central}: session {id}: channel handshake failed: {e}");
        count.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    info!("{central}: session {id}: SSH session started");
    let stopping = async {
        let _ = shutdown.wait_for(|s| *s).await;
    };
    let result = relay::run(stream, local, corebluetooth::is_link_end, stopping).await;
    let (end, from_client, to_client) = match &result {
        Ok(o) => (
            match &o.end {
                End::Local => "server ended the session".to_owned(),
                End::Peer(None) => "client closed the channel".to_owned(),
                End::Peer(Some(e)) => format!("client disconnected: {e}"),
                End::Cancelled => "server is shutting down".to_owned(),
            },
            o.from_peer,
            o.to_peer,
        ),
        Err(e) => (format!("channel error: {e}"), 0, 0),
    };
    info!(
        "{central}: session {id}: ended after {}s, {from_client} bytes from client, {to_client} bytes to client: {end}",
        started.elapsed().as_secs()
    );
    count.fetch_sub(1, Ordering::SeqCst);
}
