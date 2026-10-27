//! macOS server: CoreBluetooth through blessh-ble. Runs in the user's
//! session (macOS grants Bluetooth to logged-in users, not to daemons).
//! Every L2CAP channel becomes one connection to the launchd session
//! socket, where sshd runs as root, or one run of a command for tests.

use std::{
    error::Error,
    io,
    path::PathBuf,
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
    logging,
    protocol::refusal_line,
    relay::{self, End},
    session::{self, Preamble},
};
use clap::Parser;
use log::{error, info, warn};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::{mpsc, oneshot, watch},
};

#[derive(Parser, Debug, Clone)]
#[command(version, about = "SSH over Bluetooth LE: CoreBluetooth server")]
struct Args {
    /// Advertised name (default: host name).
    #[arg(long)]
    name: Option<String>,
    /// Stream socket that starts a session for every connection (launchd).
    #[arg(long, default_value = "/var/run/blessh-session.sock")]
    session_socket: PathBuf,
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

/// How long Bluetooth may take to come up and the service to register.
const START_TIMEOUT: Duration = Duration::from_secs(15);

pub fn main() {
    logging::init("info");
    let args = Args::parse();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(e) = rt.block_on(serve(args)) {
        error!("{e}");
        std::process::exit(1);
    }
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

async fn serve(args: Args) -> Result<(), Box<dyn Error>> {
    let name = args.name.clone().unwrap_or_else(host_name);
    match std::fs::read_to_string(&args.host_key) {
        Ok(k) => info!("SSH host key: {}", k.trim()),
        Err(e) => warn!("cannot read host key {}: {e}", args.host_key.display()),
    }
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
    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
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

/// The session's local end: the session socket (with the preamble) or a
/// command's stdio. The child, if any, is returned so that it lives as long
/// as the session.
async fn local_end(
    args: &Args,
    preamble: &Preamble,
) -> io::Result<(Local, Option<tokio::process::Child>)> {
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
    let mut s = UnixStream::connect(&args.session_socket).await?;
    s.write_all(preamble.encode().as_bytes()).await?;
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
    if count.fetch_add(1, Ordering::SeqCst) >= args.max_sessions {
        count.fetch_sub(1, Ordering::SeqCst);
        info!(
            "{central}: session {id}: refused, {} sessions in progress",
            args.max_sessions
        );
        let _ = stream
            .write_all(refusal_line(&format!("busy ({} sessions)", args.max_sessions)).as_bytes())
            .await;
        let _ = stream.shutdown().await;
        return;
    }
    let preamble = Preamble {
        id: id.clone(),
        device: central.clone(),
        address_type: "unknown".into(),
        transport: "l2cap".into(),
    };
    let (local, _child) = match local_end(&args, &preamble).await {
        Ok(l) => l,
        Err(e) => {
            error!("{central}: session {id}: cannot start session: {e}");
            let _ = stream
                .write_all(refusal_line("session unavailable").as_bytes())
                .await;
            let _ = stream.shutdown().await;
            count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
    };
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
