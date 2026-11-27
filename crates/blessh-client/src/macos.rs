//! macOS client: CoreBluetooth through blessh-ble. Servers are selected by
//! advertised name or by CoreBluetooth's device identifier (macOS shows no
//! Bluetooth addresses); sessions run over the L2CAP channel.

use std::{
    collections::HashSet,
    error::Error,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use blessh_ble::corebluetooth::{
    self,
    central::{Central, Found},
    stream::L2capStream,
};
use blessh_core::{
    bridge, channel, name,
    protocol::{self, printable, INFO_UUID},
    relay::{self, End},
    resume,
};
use log::{info, warn};
use tokio::sync::oneshot;

use crate::{
    cli::{first_options, resume_config, resumed_exit_code, stop_signal, ProxyArgs, ScanArgs},
    listing::{Printer, Row},
};

/// How long Bluetooth may take to come up.
const BLUETOOTH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long connecting, discovery, a read or opening the channel may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn scan(args: ScanArgs) -> i32 {
    match scan_sync(&args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("blessh: {e}");
            1
        }
    }
}

/// CoreBluetooth's RSSI when it has none.
const NO_RSSI: i64 = 127;

/// How long the scan waits for Bluetooth while macOS asks the user for
/// the permission.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);

fn scan_sync(args: &ScanArgs) -> Result<()> {
    let timeout = if corebluetooth::permission() == corebluetooth::Permission::Undecided {
        eprintln!(
            "blessh: waiting for Bluetooth permission: answer macOS's prompt on this Mac's screen"
        );
        PERMISSION_TIMEOUT
    } else {
        BLUETOOTH_TIMEOUT
    };
    let central = Central::new(timeout)?;
    let mut printer = Printer::new(args.format);
    // A server is listed once its advertised name is known; those whose
    // scan response never came are listed at the end with the name
    // CoreBluetooth has cached.
    let mut listed = HashSet::new();
    let mut pending: Vec<(String, String, i64)> = Vec::new();
    central.scan(args.duration(), |f| {
        if listed.contains(&f.id) || f.rssi == NO_RSSI {
            return false;
        }
        match &f.local_name {
            Some(n) => {
                printer.row(&Row::new(n, rssi(f.rssi), &f.id));
                listed.insert(f.id.clone());
                pending.retain(|(id, _, _)| *id != f.id);
            }
            None if !pending.iter().any(|(id, _, _)| *id == f.id) => {
                pending.push((
                    f.id.clone(),
                    f.name().unwrap_or_default().to_owned(),
                    f.rssi,
                ));
            }
            None => {}
        }
        false
    })?;
    for (id, n, r) in pending {
        printer.row(&Row::new(&n, rssi(r), &id));
    }
    Ok(())
}

fn rssi(r: i64) -> i16 {
    r.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

pub fn proxy(args: ProxyArgs) -> i32 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match rt.block_on(proxy_async(args)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("blessh: {e}");
            1
        }
    };
    // A read of stdin may still be blocked in a runtime thread; waiting for
    // it would keep the process alive after the session.
    rt.shutdown_background();
    code
}

/// Connects to a server and reads its INFO.
fn read_info(central: &Central, found: &Found) -> io::Result<String> {
    central.connect(&found.peripheral, STEP_TIMEOUT)?;
    let chars = central.discover(&found.peripheral, STEP_TIMEOUT)?;
    let info_char = chars
        .iter()
        .find(|(uuid, _)| *uuid == INFO_UUID.to_string())
        .map(|(_, c)| c.clone())
        .ok_or_else(|| io::Error::other("device does not provide the blessh service"))?;
    Ok(
        String::from_utf8_lossy(&central.read(&found.peripheral, &info_char, STEP_TIMEOUT)?)
            .into_owned(),
    )
}

/// A server this Mac is connected to already, which may not advertise
/// while the link is up: selected by identifier or by the name in its INFO.
fn connected_server(central: &Central, target: Option<&str>) -> Option<(Found, String)> {
    for f in central.connected() {
        let by_id = target.map_or(true, |t| f.id.eq_ignore_ascii_case(t));
        match read_info(central, &f) {
            Ok(info)
                if by_id
                    || protocol::info_value(&info, "name")
                        .is_some_and(|n| target.is_some_and(|t| name::same(n, t))) =>
            {
                return Some((f, info));
            }
            Ok(_) => {}
            Err(e) => log::debug!("connected device {}: {e}", f.id),
        }
    }
    None
}

/// How long the scan looks for a server by name before the client tries
/// the device it had under that name last time.
const QUICK_SCAN: Duration = Duration::from_secs(5);

/// A server found by a short scan or, failing that, the device that had
/// its name last time, connected without waiting for its advertisement (a
/// server may not advertise right after a link went down).
fn known_server(
    central: &Central,
    target: Option<&str>,
    scan_timeout: Duration,
) -> Option<(Found, String)> {
    let t = target?;
    let id = crate::cache::device_of(t)?;
    if let Ok(found) = scan_for(central, Some(t), QUICK_SCAN.min(scan_timeout)) {
        return read_info(central, &found).ok().map(|info| (found, info));
    }
    let found = central.known(&id)?;
    match read_info(central, &found) {
        Ok(info) if protocol::info_value(&info, "name").is_some_and(|n| name::same(n, t)) => {
            log::debug!("{t}: connected to the known device {id}");
            Some((found, info))
        }
        Ok(_) => None,
        Err(e) => {
            log::debug!("{t}: known device {id}: {e}");
            None
        }
    }
}

/// The server on device `id`, which a scan just saw (`--device`):
/// connected without scanning again when CoreBluetooth knows it.
fn on_device(central: &Central, id: &str, scan_timeout: Duration) -> io::Result<(Found, String)> {
    if let Some(c) = connected_server(central, Some(id)) {
        return Ok(c);
    }
    if let Some(found) = central.known(id) {
        match read_info(central, &found) {
            Ok(info) => return Ok((found, info)),
            Err(e) => log::debug!("known device {id}: {e}"),
        }
    }
    let found = scan_for(central, Some(id), scan_timeout)?;
    let info = read_info(central, &found)?;
    Ok((found, info))
}

/// Finds the server, connects and reads INFO. CoreBluetooth calls block,
/// so this runs off the async runtime.
fn open(
    target: Option<String>,
    device: Option<String>,
    scan_timeout: Duration,
) -> io::Result<(Central, Found, Vec<u16>)> {
    let central = Central::new(BLUETOOTH_TIMEOUT)?;
    let (found, info) = match &device {
        Some(id) => on_device(&central, id, scan_timeout)?,
        None => match connected_server(&central, target.as_deref()) {
            Some(c) => c,
            None => match known_server(&central, target.as_deref(), scan_timeout) {
                Some(k) => k,
                None => {
                    let found = scan_for(&central, target.as_deref(), scan_timeout)?;
                    let info = read_info(&central, &found)?;
                    (found, info)
                }
            },
        },
    };
    if let (Some(id), Some(t)) = (&device, &target) {
        let n = protocol::info_value(&info, "name").unwrap_or_default();
        if !name::same(n, t) {
            central.disconnect(&found.peripheral);
            return Err(io::Error::other(format!(
                "{id} is now the server {n:?}, not {t:?}"
            )));
        }
    }
    if let Some(n) = protocol::info_value(&info, "name") {
        crate::cache::put_device(n, &found.id);
    }
    info!("server info: {}", printable(&info));
    let psms = protocol::info_psms(&info);
    if psms.is_empty() {
        return Err(io::Error::other(
            "the server offers no L2CAP channel; the GATT stream is not available on macOS yet",
        ));
    }
    Ok((central, found, psms))
}

fn scan_for(central: &Central, target: Option<&str>, scan_timeout: Duration) -> io::Result<Found> {
    // Servers in range by identifier, with the best name known so far.
    let mut seen: Vec<(String, String)> = Vec::new();
    central
        .scan(scan_timeout, |f| {
            let n = f.name().unwrap_or_default().to_owned();
            match seen.iter_mut().find(|(id, _)| *id == f.id) {
                Some(e) if f.local_name.is_some() => e.1 = n,
                Some(_) => {}
                None => seen.push((f.id.clone(), n)),
            }
            // Only the advertised name counts: the cached one may be stale.
            match target {
                None => true,
                Some(t) => {
                    f.id.eq_ignore_ascii_case(t)
                        || f.local_name.as_deref().is_some_and(|n| name::same(n, t))
                }
            }
        })?
        .ok_or_else(|| {
            let what = match target {
                Some(t) => format!("no blessh server named {t:?} found"),
                None => "no blessh server found".into(),
            };
            let names: Vec<&str> = seen
                .iter()
                .map(|(_, n)| n.as_str())
                .filter(|n| !n.is_empty())
                .collect();
            if names.is_empty() {
                io::Error::other(what)
            } else {
                io::Error::other(format!("{what}; servers in range: {}", names.join(", ")))
            }
        })
}

/// Opens the first of the server's channels that passes the handshake,
/// with `options` in the hello. CoreBluetooth calls block, so they run off
/// the async runtime; the central travels with them and comes back in
/// every case.
async fn open_channel(
    mut central: Central,
    found: &Found,
    psms: &[u16],
    args: &ProxyArgs,
    options: &channel::Options,
) -> (Central, Result<(L2capStream, u16, channel::Options)>) {
    let attempts = crate::cache::order(&found.id, psms.iter().map(|p| (*p, 0)).collect());
    let _opening = crate::cache::OpenLock::take(&found.id).await;
    for (i, &(psm, _)) in attempts.iter().enumerate() {
        let next = if i + 1 < attempts.len() {
            "trying the next channel"
        } else {
            "no channel left"
        };
        let peripheral = found.peripheral.clone();
        let (c, opened) = match tokio::task::spawn_blocking(move || {
            let r = central.open_channel(&peripheral, psm, STEP_TIMEOUT);
            (central, r)
        })
        .await
        {
            Ok(v) => v,
            Err(e) => panic!("CoreBluetooth call panicked: {e}"),
        };
        central = c;
        let mut stream = match opened {
            Ok(s) => s,
            Err(e) => {
                warn!("L2CAP channel on PSM {psm}: {e}; {next}");
                continue;
            }
        };
        match channel::client_with(
            &mut stream,
            args.probe,
            Duration::from_secs(args.stall_timeout),
            options,
        )
        .await
        {
            Ok(ok) => {
                log::debug!(
                    "channel on PSM {psm}: handshake version {} in {} ms",
                    ok.version,
                    ok.elapsed.as_millis()
                );
                crate::cache::put(&found.id, psm, 0);
                return (central, Ok((stream, psm, ok.options)));
            }
            Err(channel::ClientError::Refused(r)) => return (central, Err(r.into())),
            Err(e @ channel::ClientError::CannotResume(_)) => return (central, Err(Box::new(e))),
            Err(e) => warn!("L2CAP channel on PSM {psm}: {e}; {next}"),
        }
    }
    (
        central,
        Err(
            "no L2CAP channel of the server works, and the GATT stream is not available on macOS yet"
                .into(),
        ),
    )
}

async fn proxy_async(args: ProxyArgs) -> Result<i32> {
    if args.gatt {
        return Err("the GATT stream is not available on macOS yet".into());
    }
    let (target, device) = (args.target.clone(), args.device.clone());
    let scan_timeout = Duration::from_secs(args.scan_timeout);
    let (central, found, psms) =
        tokio::task::spawn_blocking(move || open(target, device, scan_timeout)).await??;
    let (central, opened) =
        open_channel(central, &found, &psms, &args, &first_options(&args)).await;
    let (stream, psm, reply) = opened?;
    eprintln!(
        "blessh: {} {}: l2cap, psm {psm}",
        found.name().unwrap_or_default(),
        found.id
    );
    match resume::parse_session(&reply).filter(|_| args.resume > 0) {
        Some(grant) => resumable_session(central, found, psms, stream, grant, &args).await,
        None => plain_session(central, found, stream, &args).await,
    }
}

/// Relays stdin/stdout over a channel without a resumable session.
async fn plain_session(
    central: Central,
    found: Found,
    stream: L2capStream,
    args: &ProxyArgs,
) -> Result<i32> {
    let stdio = bridge::stdio()?;
    // The link going down ends the session even when the channel's streams
    // do not report it.
    let stop = Arc::new(AtomicBool::new(false));
    let watch_stop = stop.clone();
    let watched = found.peripheral.clone();
    let watcher = tokio::task::spawn_blocking(move || {
        let lost = central.wait_disconnected(&watched, &watch_stop);
        (central, lost)
    });
    tokio::pin!(watcher);
    let relay = relay::run(stream, stdio, corebluetooth::is_link_end, stop_signal()?);
    let (outcome, central) = tokio::select! {
        outcome = relay => {
            stop.store(true, Ordering::SeqCst);
            let (central, _) = (&mut watcher).await?;
            (outcome, central)
        }
        watched = &mut watcher => {
            let (_, lost) = watched?;
            warn!("link lost: {}", lost.unwrap_or_default());
            return Ok(3);
        }
    };
    if !args.keep_link {
        central.disconnect(&found.peripheral);
    }
    let outcome = outcome?;
    log::debug!(
        "session ended: {:?}, {} bytes from the server, {} bytes to it",
        outcome.end,
        outcome.from_peer,
        outcome.to_peer
    );
    Ok(match outcome.end {
        End::Local | End::Cancelled | End::Peer(None) => 0,
        End::Peer(Some(e)) => {
            warn!("channel ended: {e}");
            3
        }
    })
}

/// A thread that holds the central while it waits for the link to go
/// down, and fires `lost` when it does.
struct LinkWatch {
    stop: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<Central>,
}

impl LinkWatch {
    fn start(central: Central, found: &Found) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let watch_stop = stop.clone();
        let watched = found.peripheral.clone();
        let task = tokio::task::spawn_blocking(move || {
            if let Some(why) = central.wait_disconnected(&watched, &watch_stop) {
                log::debug!("link lost: {why}");
                let _ = tx.send(());
            }
            central
        });
        (Self { stop, task }, rx)
    }

    /// Stops watching and gives the central back.
    async fn central(self) -> Central {
        self.stop.store(true, Ordering::SeqCst);
        match self.task.await {
            Ok(c) => c,
            Err(e) => panic!("link watcher panicked: {e}"),
        }
    }
}

/// Where the central is between channels.
enum Held {
    Idle(Central),
    Watching(LinkWatch),
}

impl Held {
    async fn central(self) -> Central {
        match self {
            Held::Idle(c) => c,
            Held::Watching(w) => w.central().await,
        }
    }
}

/// Runs a resumable session: when the channel or the link is lost, ssh's
/// stdio stays open while the client connects again and resumes the
/// session on a new channel.
async fn resumable_session(
    central: Central,
    found: Found,
    psms: Vec<u16>,
    stream: L2capStream,
    grant: resume::Grant,
    args: &ProxyArgs,
) -> Result<i32> {
    log::debug!(
        "resumable session {}…, server replay buffer {}",
        resume::token_tag(&grant.token),
        grant.replay
    );
    let mut stdio = bridge::stdio()?;
    let (watch, lost) = LinkWatch::start(central, &found);
    let first = resume::Watched::new(stream, lost);
    let held = Arc::new(tokio::sync::Mutex::new(Some(Held::Watching(watch))));
    let (found, psms) = (&found, &psms);
    let reconnect = |req: resume::ResumeRequest| {
        let held = held.clone();
        async move {
            let mut slot = held.lock().await;
            let Some(h) = slot.take() else {
                return resume::Reconnect::CannotResume("no Bluetooth central".into());
            };
            let central = h.central().await;
            let peripheral = found.peripheral.clone();
            let (central, connected) = match tokio::task::spawn_blocking(move || {
                let r = if central.is_connected(&peripheral) {
                    Ok(())
                } else {
                    central.connect(&peripheral, STEP_TIMEOUT)
                };
                (central, r)
            })
            .await
            {
                Ok(v) => v,
                Err(e) => panic!("CoreBluetooth call panicked: {e}"),
            };
            if let Err(e) = connected {
                *slot = Some(Held::Idle(central));
                return resume::Reconnect::Failed(e.to_string());
            }
            let opts = resume::client_options(resume::DEFAULT_REPLAY, Some(&req));
            let (central, opened) = open_channel(central, found, psms, args, &opts).await;
            match opened {
                Ok((stream, _psm, reply)) => match resume::parse_session(&reply) {
                    Some(g) => {
                        let (watch, lost) = LinkWatch::start(central, found);
                        *slot = Some(Held::Watching(watch));
                        resume::Reconnect::Attached(resume::Watched::new(stream, lost), g)
                    }
                    None => {
                        *slot = Some(Held::Idle(central));
                        resume::Reconnect::CannotResume("no session in the reply".into())
                    }
                },
                Err(e) => {
                    *slot = Some(Held::Idle(central));
                    match e.downcast_ref::<channel::ClientError>() {
                        Some(channel::ClientError::CannotResume(r)) => {
                            resume::Reconnect::CannotResume(r.clone())
                        }
                        _ => resume::Reconnect::Failed(e.to_string()),
                    }
                }
            }
        }
    };
    let summary = resume::run_client(
        &mut stdio,
        first,
        grant,
        resume_config(args),
        reconnect,
        stop_signal()?,
        |line| info!("{line}"),
    )
    .await;
    if let Some(h) = held.lock().await.take() {
        let central = h.central().await;
        if !args.keep_link {
            central.disconnect(&found.peripheral);
        }
    }
    Ok(resumed_exit_code(&summary))
}
