//! ble-term-daemon: exposes the BleStream GATT service through BlueZ and
//! forwards each BLE client to a session socket (normally a systemd socket
//! that spawns `sshd -i` per connection).

use std::{path::PathBuf, sync::Arc, time::Duration};

use ble_term::{
    bridge::{self, BridgeError, Outcome, PacketTx},
    link::Link,
    logging,
    protocol::{self, printable, CloseReason, Frame, INFO_UUID, RX_UUID, SERVICE_UUID, TX_UUID},
    session::{self, Preamble},
};
use bluer::{
    adv::{Advertisement, AdvertisementHandle},
    gatt::{
        local::{
            characteristic_control, Application, Characteristic, CharacteristicControlEvent,
            CharacteristicNotify, CharacteristicNotifyMethod, CharacteristicRead,
            CharacteristicWrite, CharacteristicWriteMethod, ReqError, Service,
        },
        CharacteristicReader, CharacteristicWriter,
    },
    Adapter, Address, AddressType, DeviceEvent, DeviceProperty,
};
use clap::Parser;
use futures::{FutureExt, StreamExt};
use log::{debug, error, info, warn};
use tokio::{io::AsyncWriteExt, net::UnixStream, sync::mpsc, task::JoinHandle, time::Instant};

#[derive(Parser, Debug, Clone)]
#[command(version, about = "SSH over Bluetooth LE: BlueZ GATT server")]
struct Args {
    /// Bluetooth adapter to use (default: BlueZ default adapter).
    #[arg(long)]
    adapter: Option<String>,
    /// Advertised name (default: host name).
    #[arg(long)]
    name: Option<String>,
    /// Stream socket that starts a session for every connection.
    #[arg(long, default_value = "/run/ble-term/session.sock")]
    session_socket: PathBuf,
    /// Seconds a client may take to complete the BleStream handshake.
    #[arg(long, default_value_t = 10)]
    handshake_timeout: u64,
    /// Receive window granted to the client, in bytes.
    #[arg(long, default_value_t = 16384)]
    window: u32,
    /// Re-register the advertisement after each session (for controllers
    /// that do not resume advertising after a connection).
    #[arg(long)]
    readvertise: bool,
    /// SSH host public key whose fingerprint is logged at start-up.
    #[arg(long, default_value = "/etc/ble-term/ssh_host_ed25519_key.pub")]
    host_key: PathBuf,
}

/// Characteristics opened by a client before both are there.
struct Pending {
    id: u64,
    reader: Option<CharacteristicReader>,
    writer: Option<Arc<CharacteristicWriter>>,
    since: Instant,
    /// Reports the end of the link the sockets belong to (see `watch`).
    watcher: Option<JoinHandle<()>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(w) = self.watcher.take() {
            w.abort();
        }
    }
}

impl Pending {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            reader: None,
            writer: None,
            since: Instant::now(),
            watcher: None,
        }
    }

    /// Called when the first socket arrives. A socket belongs to one link:
    /// one acquired just before a disconnect (a late write of the previous
    /// session) must not be paired with a socket of the next link, so the
    /// pending state is dropped when this link goes down.
    fn watch(&mut self, adapter: &Adapter, addr: Address, gone: &mpsc::UnboundedSender<u64>) {
        let (adapter, gone, id) = (adapter.clone(), gone.clone(), self.id);
        self.watcher = Some(tokio::spawn(async move {
            disconnected(&adapter, addr).await;
            let _ = gone.send(id);
        }));
    }

    fn address(&self) -> Option<Address> {
        self.reader
            .as_ref()
            .map(|r| r.device_address())
            .or(self.writer.as_ref().map(|w| w.device_address()))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    logging::init("info");
    let args = Args::parse();
    if let Err(e) = serve(args).await {
        error!("{e}");
        std::process::exit(1);
    }
}

fn host_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "ble-term".into())
}

/// Why an adapter is off, with the command that fixes it.
fn power_hint(adapter: &str) -> String {
    let rfkill = std::fs::read_dir(format!("/sys/class/bluetooth/{adapter}"))
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("rfkill"))
        .map(|e| e.path());
    let flag = |name: &str| {
        rfkill
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p.join(name)).ok())
            .is_some_and(|v| v.trim() == "1")
    };
    if flag("hard") {
        "it is blocked by a hardware switch or the firmware (rfkill hard block)".into()
    } else if flag("soft") {
        "it is blocked by rfkill; unblock it with `sudo rfkill unblock bluetooth`".into()
    } else {
        "switch it on with `bluetoothctl power on`".into()
    }
}

async fn wait_adapter(session: &bluer::Session, name: Option<&str>) -> bluer::Result<Adapter> {
    let mut warned = false;
    loop {
        let found = match name {
            Some(n) => session
                .adapter_names()
                .await?
                .iter()
                .any(|a| a == n)
                .then(|| session.adapter(n))
                .transpose()?,
            None => session.default_adapter().await.ok(),
        };
        if let Some(a) = found {
            return Ok(a);
        }
        if !warned {
            warn!(
                "Bluetooth adapter {} not found, waiting for it to appear",
                name.unwrap_or("(any)")
            );
            warned = true;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn wait_powered(adapter: &Adapter) -> bluer::Result<()> {
    let mut last_hint = None;
    while !adapter.is_powered().await? {
        let hint = power_hint(adapter.name());
        if last_hint.as_ref() != Some(&hint) {
            warn!("Bluetooth adapter {} is off: {hint}", adapter.name());
            last_hint = Some(hint);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    if last_hint.is_some() {
        info!("Bluetooth adapter {} is on", adapter.name());
    }
    Ok(())
}

const HALF_OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// How a session ended, for the main loop.
struct SessionEnd {
    /// Disconnect the device after a grace period.
    disconnect: bool,
    /// Notification socket to keep for a reconnect over the same link.
    notify: Option<Arc<CharacteristicWriter>>,
}

const ADV_INTERVAL: Duration = Duration::from_millis(400);

async fn advertise(adapter: &Adapter, name: &str) -> bluer::Result<AdvertisementHandle> {
    let adv = Advertisement {
        service_uuids: [SERVICE_UUID].into_iter().collect(),
        discoverable: Some(true),
        local_name: Some(name.to_owned()),
        // BlueZ defaults to 1.28 s; a phone can only start connecting after it
        // hears an advertisement, so a shorter interval makes connects faster.
        min_interval: Some(ADV_INTERVAL),
        max_interval: Some(ADV_INTERVAL),
        ..Default::default()
    };
    adapter.advertise(adv).await
}

async fn serve(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let session = bluer::Session::new().await?;
    let adapter = wait_adapter(&session, args.adapter.as_deref()).await?;
    wait_powered(&adapter).await?;
    let name = args.name.clone().unwrap_or_else(host_name);
    info!(
        "adapter {} ({}), advertising as {:?}",
        adapter.name(),
        adapter.address().await?,
        name
    );
    match std::fs::read_to_string(&args.host_key) {
        Ok(k) => info!("SSH host key: {}", k.trim()),
        Err(e) => warn!("cannot read host key {}: {e}", args.host_key.display()),
    }

    let info_name = name.clone();
    let (rx_control, rx_handle) = characteristic_control();
    let (tx_control, tx_handle) = characteristic_control();
    let app = Application {
        services: vec![Service {
            uuid: SERVICE_UUID,
            primary: true,
            characteristics: vec![
                Characteristic {
                    uuid: INFO_UUID,
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(move |req| {
                            let v = protocol::info(&info_name, req.mtu).into_bytes();
                            let part = protocol::read_at(&v, req.offset)
                                .map(<[u8]>::to_vec)
                                .ok_or(ReqError::InvalidOffset);
                            async move { part }.boxed()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Characteristic {
                    uuid: RX_UUID,
                    write: Some(CharacteristicWrite {
                        write: true,
                        write_without_response: true,
                        method: CharacteristicWriteMethod::Io,
                        ..Default::default()
                    }),
                    control_handle: rx_handle,
                    ..Default::default()
                },
                Characteristic {
                    uuid: TX_UUID,
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: CharacteristicNotifyMethod::Io,
                        ..Default::default()
                    }),
                    control_handle: tx_handle,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let _app = adapter.serve_gatt_application(app).await?;
    let mut adv = Some(advertise(&adapter, &name).await?);
    info!("ready (BlueZ GATT application and advertisement registered)");

    tokio::pin!(rx_control, tx_control);
    let handshake_timeout = Duration::from_secs(args.handshake_timeout);
    let mut pending = Pending::new();
    let (gone_tx, mut gone_rx) = mpsc::unbounded_channel::<u64>();
    let mut active: Option<(Address, JoinHandle<SessionEnd>)> = None;
    // Notification socket of a session that ended cleanly while its link
    // stays up. The client may reuse the link right away; it then rewrites
    // the CCCD with the value it already has, and BlueZ < 5.69 does not call
    // AcquireNotify again, so the next session needs this socket.
    let mut parked: Option<Arc<CharacteristicWriter>> = None;
    // Device to disconnect once its session has ended, unless it reconnects first.
    let mut deferred_disconnect: Option<(Address, Instant)> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            ev = rx_control.next() => match ev {
                Some(CharacteristicControlEvent::Write(req)) => {
                    let addr = req.device_address();
                    if let Some(other) = other_client(&adapter, addr, &mut pending, &mut active).await {
                        info!("{addr}: rejecting, another client ({other}) is connected");
                        req.reject(ReqError::NotPermitted);
                        disconnect(&adapter, addr);
                    } else {
                        if let Some((_, task)) = active.take() {
                            info!("{addr}: client reopened the stream, replacing its previous session");
                            task.abort();
                        }
                        debug!("{addr}: write acquired, MTU {}", req.mtu());
                        if pending.address().is_none() {
                            pending.since = Instant::now();
                            pending.watch(&adapter, addr, &gone_tx);
                        }
                        pending.reader = Some(req.accept()?);
                        match parked.take() {
                            Some(w) if w.device_address() == addr && pending.writer.is_none() => {
                                if w.closed().now_or_never().is_none() {
                                    debug!("{addr}: reusing the notification socket of its previous session");
                                    pending.writer = Some(w);
                                }
                            }
                            Some(w) if w.device_address() != addr => {
                                // Its socket would keep BlueZ < 5.69 from handing
                                // one to the new client; let the old link go.
                                disconnect(&adapter, w.device_address());
                            }
                            _ => {}
                        }
                    }
                }
                Some(_) => {}
                None => return Err("GATT application was unregistered".into()),
            },
            ev = tx_control.next() => match ev {
                Some(CharacteristicControlEvent::Notify(writer)) => {
                    let addr = writer.device_address();
                    if let Some(other) = other_client(&adapter, addr, &mut pending, &mut active).await {
                        // Only BlueZ >= 5.69 hands out per-device notify sockets,
                        // so this cannot reach the active client.
                        info!("{addr}: rejecting, another client ({other}) is connected");
                        let busy = Frame::Close { reason: CloseReason::Busy, message: "another client is connected".into() };
                        let _ = writer.try_send(&busy.encode(0));
                        drop(writer);
                        disconnect(&adapter, addr);
                    } else {
                        if let Some((_, task)) = active.take() {
                            info!("{addr}: client resubscribed, replacing its previous session");
                            task.abort();
                        }
                        debug!("{addr}: notifications enabled, MTU {}", writer.mtu());
                        if pending.address().is_none() {
                            pending.since = Instant::now();
                            pending.watch(&adapter, addr, &gone_tx);
                        }
                        if parked.as_ref().is_some_and(|w| w.device_address() == addr) {
                            parked = None;
                        }
                        pending.writer = Some(Arc::new(writer));
                    }
                }
                Some(_) => {}
                None => return Err("GATT application was unregistered".into()),
            },
            Some(id) = gone_rx.recv() => {
                if id == pending.id {
                    if let Some(addr) = pending.address() {
                        info!("{addr}: link went down, dropping its half-opened stream");
                    }
                    pending = Pending::new();
                }
            }
            done = async { (&mut active.as_mut().unwrap().1).await }, if active.is_some() => {
                let (addr, _) = active.take().unwrap();
                debug!("{addr}: session task finished");
                let done = done.unwrap_or(SessionEnd { disconnect: false, notify: None });
                parked = done.notify;
                if done.disconnect {
                    // Let the client read the CLOSE frame and disconnect by
                    // itself. Phones keep the link for a while and may reuse it
                    // when the user reconnects right away; the delay keeps the
                    // window in which that races with our disconnect small.
                    deferred_disconnect = Some((addr, Instant::now() + Duration::from_secs(10)));
                }
                // The kernel resumes advertising on disconnect; re-registering
                // here raced with reconnecting clients in tests, so it is opt-in.
                if args.readvertise {
                    adv = None;
                    match advertise(&adapter, &name).await {
                        Ok(h) => adv = Some(h),
                        Err(e) => error!("re-registering advertisement failed: {e}"),
                    }
                }
            }
            _ = tick.tick() => {
                if adv.is_none() {
                    match advertise(&adapter, &name).await {
                        Ok(h) => adv = Some(h),
                        Err(e) => debug!("advertising retry failed: {e}"),
                    }
                }
                if let Some((addr, at)) = deferred_disconnect {
                    if Instant::now() >= at {
                        deferred_disconnect = None;
                        if pending.address() != Some(addr) && active.as_ref().map(|(a, _)| *a) != Some(addr) {
                            if parked.as_ref().is_some_and(|w| w.device_address() == addr) {
                                parked = None;
                            }
                            disconnect(&adapter, addr);
                        }
                    }
                }
                if let Some(addr) = pending.address() {
                    if !is_connected(&adapter, addr).await {
                        info!("{addr}: disconnected before opening both characteristics");
                        pending = Pending::new();
                    } else if (pending.reader.is_some() != pending.writer.is_some())
                        && pending.since.elapsed() > HALF_OPEN_TIMEOUT
                    {
                        // One characteristic opened, the other not: with
                        // BlueZ < 5.69 the second one may never come on this
                        // link. A fresh connection fixes it.
                        info!("{addr}: client opened only one characteristic, dropping the link");
                        pending = Pending::new();
                        disconnect(&adapter, addr);
                    } else if pending.since.elapsed() > handshake_timeout {
                        info!("{addr}: client did not open both characteristics in time");
                        pending = Pending::new();
                        disconnect(&adapter, addr);
                    }
                }
            }
        }

        if active.is_none() && pending.reader.is_some() && pending.writer.is_some() {
            let mut p = std::mem::replace(&mut pending, Pending::new());
            let (reader, writer) = (p.reader.take().unwrap(), p.writer.take().unwrap());
            let addr = reader.device_address();
            if writer.device_address() != addr {
                warn!(
                    "characteristics opened by different devices ({addr}, {}), dropping both",
                    writer.device_address()
                );
                continue;
            }
            let adapter = adapter.clone();
            let args = args.clone();
            active = Some((
                addr,
                tokio::spawn(async move { run_session(adapter, addr, reader, writer, args).await }),
            ));
        }
    }

    info!("shutting down");
    if let Some((_, task)) = active {
        task.abort();
    }
    drop(adv);
    Ok(())
}

/// Disconnects a device in the background. Device1.Disconnect can take
/// seconds; awaiting it in the main loop would delay the replies to
/// AcquireWrite/AcquireNotify of a reconnecting client until its link is gone.
fn disconnect(adapter: &Adapter, addr: Address) {
    let adapter = adapter.clone();
    tokio::spawn(async move {
        if let Ok(dev) = adapter.device(addr) {
            if let Err(e) = dev.disconnect().await {
                debug!("{addr}: disconnect failed: {e}");
            }
        }
    });
}

async fn is_connected(adapter: &Adapter, addr: Address) -> bool {
    match adapter.device(addr) {
        Ok(dev) => dev.is_connected().await.unwrap_or(false),
        Err(_) => false,
    }
}

/// Returns the other device that holds the server, if any. State left behind
/// by a device that is no longer connected (a half-opened stream, a session
/// whose link loss has not been noticed yet) is dropped instead of blocking
/// the new client.
async fn other_client(
    adapter: &Adapter,
    addr: Address,
    pending: &mut Pending,
    active: &mut Option<(Address, JoinHandle<SessionEnd>)>,
) -> Option<Address> {
    if let Some(other) = pending.address().filter(|a| *a != addr) {
        if is_connected(adapter, other).await {
            return Some(other);
        }
        info!("{other}: no longer connected, dropping its half-opened stream");
        *pending = Pending::new();
    }
    if let Some(other) = active.as_ref().map(|(a, _)| *a).filter(|a| *a != addr) {
        if is_connected(adapter, other).await {
            return Some(other);
        }
        info!("{other}: no longer connected, ending its session");
        if let Some((_, task)) = active.take() {
            task.abort();
        }
    }
    None
}

/// Resolves when the device is not connected (now or later).
async fn disconnected(adapter: &Adapter, addr: Address) {
    let Ok(dev) = adapter.device(addr) else {
        return;
    };
    let Ok(mut events) = dev.events().await else {
        return;
    };
    if !dev.is_connected().await.unwrap_or(false) {
        return;
    }
    while let Some(DeviceEvent::PropertyChanged(p)) = events.next().await {
        if let DeviceProperty::Connected(false) = p {
            return;
        }
    }
}

/// Resolves when the device is no longer connected or the notify socket closes.
async fn link_gone(adapter: Adapter, addr: Address, writer: Arc<CharacteristicWriter>) {
    tokio::select! {
        _ = disconnected(&adapter, addr) => {}
        _ = writer.closed() => {}
    }
}

async fn address_type(adapter: &Adapter, addr: Address) -> &'static str {
    let t = match adapter.device(addr) {
        Ok(dev) => dev.address_type().await.ok(),
        Err(_) => None,
    };
    match t {
        Some(AddressType::LePublic) => "public",
        Some(AddressType::LeRandom) => "random",
        _ => "unknown",
    }
}

/// Runs one session; returns true if the device should be disconnected.
///
/// Every session gets a random id. The daemon logs it with the device
/// address when the session starts and with duration, byte counts and the
/// way it ended when it is over; ble-term-session logs the same id in the
/// journal of the sshd it starts, which ties logins to BLE devices.
async fn run_session(
    adapter: Adapter,
    addr: Address,
    mut reader: CharacteristicReader,
    writer: Arc<CharacteristicWriter>,
    args: Args,
) -> SessionEnd {
    let mut tx = writer.clone();
    let mut link = Link::new(writer.mtu(), args.window);
    let started = Instant::now();
    let preamble = Preamble {
        id: session::new_id(),
        device: addr.to_string(),
        address_type: address_type(&adapter, addr).await.into(),
    };
    let id = preamble.id.clone();
    // Logs the end of the session if the task is aborted (client replaced by
    // a reconnect, or its link found dead) instead of returning.
    struct AbortLog {
        addr: Address,
        id: String,
        started: Instant,
        armed: bool,
    }
    impl Drop for AbortLog {
        fn drop(&mut self) {
            if self.armed {
                info!(
                    "{}: session {}: ended after {}s: replaced or link lost",
                    self.addr,
                    self.id,
                    self.started.elapsed().as_secs()
                );
            }
        }
    }
    let mut abort_log = AbortLog {
        addr,
        id: id.clone(),
        started,
        armed: true,
    };
    info!(
        "{addr}: session {id}: client connected ({} address, notify MTU {}, write MTU {})",
        preamble.address_type,
        writer.mtu(),
        reader.mtu()
    );

    let result = async {
        let initial = bridge::handshake(
            &mut link,
            &mut reader,
            &mut tx,
            Duration::from_secs(args.handshake_timeout),
        )
        .await?;
        let stream = async {
            let mut s = UnixStream::connect(&args.session_socket).await?;
            s.write_all(preamble.encode().as_bytes()).await?;
            Ok::<_, std::io::Error>(s)
        };
        let stream = match stream.await {
            Ok(s) => s,
            Err(e) => {
                error!(
                    "{addr}: session {id}: cannot start session via {}: {e}",
                    args.session_socket.display()
                );
                link.close(CloseReason::SessionUnavailable, "cannot start session");
                while let Some(p) = link.poll_transmit() {
                    tx.send(&p).await?;
                }
                return Ok(None);
            }
        };
        info!("{addr}: session {id}: SSH session started");
        let gone = link_gone(adapter.clone(), addr, writer.clone());
        bridge::run(&mut link, &mut reader, &mut tx, stream, initial, gone)
            .await
            .map(Some)
    }
    .await;

    let clean = matches!(
        result,
        Ok(Some(Outcome::StreamEnded | Outcome::PeerClosed { .. }))
    );
    let (end, lost) = match result {
        Ok(Some(Outcome::StreamEnded)) => ("server ended the session".to_owned(), false),
        Ok(Some(Outcome::PeerClosed { reason, message })) => (
            format!(
                "client closed the session ({reason:?} {})",
                printable(&message)
            ),
            false,
        ),
        Ok(Some(Outcome::LinkLost | Outcome::Cancelled)) => ("client disconnected".into(), true),
        Ok(None) => ("session unavailable".into(), false),
        Err(BridgeError::Protocol(e)) => {
            link.close(CloseReason::ProtocolError, &e.to_string());
            while let Some(p) = link.poll_transmit() {
                if tx.send(&p).await.is_err() {
                    break;
                }
            }
            (format!("protocol error: {e}"), false)
        }
        Err(BridgeError::HandshakeTimeout) => ("handshake timed out".into(), false),
        Err(BridgeError::Io(e)) => (format!("link error: {e}"), true),
    };
    abort_log.armed = false;
    let (sent, received) = link.stats();
    info!(
        "{addr}: session {id}: ended after {}s, {received} bytes from client, {sent} bytes to client: {end}",
        started.elapsed().as_secs()
    );
    drop(tx);
    SessionEnd {
        disconnect: !lost,
        notify: clean.then_some(writer),
    }
}
