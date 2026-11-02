//! `blessh server run` on Linux: exposes the BleStream GATT service and the
//! L2CAP channels through BlueZ and forwards each BLE client to a session
//! socket (normally a systemd socket that spawns `sshd -i` per connection).

use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use blessh_ble::bluez::{self, GattRx, GattTx};
use blessh_core::{
    bridge::{self, BridgeError, Outcome, PacketTx},
    channel,
    link::Link,
    logging, name,
    protocol::{
        self, printable, CloseReason, Frame, INFO_UUID, PSM_UUID, RX_UUID, SERVICE_UUID, TX_UUID,
    },
    relay::{self, End},
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
    l2cap::{SocketAddr, Stream, StreamListener},
    Adapter, AdapterEvent, AdapterProperty, Address, AddressType, DeviceEvent, DeviceProperty,
    SessionEvent,
};
use clap::Parser;
use futures::{FutureExt, StreamExt};
use log::{debug, error, info, warn};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{mpsc, watch},
    task::JoinHandle,
    time::Instant,
};

#[derive(Parser, Debug, Clone)]
#[command(version, about = "SSH over Bluetooth LE: BlueZ GATT server")]
struct Args {
    /// Bluetooth adapter to use (default: BlueZ default adapter).
    #[arg(long)]
    adapter: Option<String>,
    /// Advertised name, by host name rules (default: the static host name
    /// from /etc/hostname, which mDNS conflicts and DHCP do not change).
    #[arg(long)]
    name: Option<String>,
    /// Print the name the server would advertise and exit.
    #[arg(long)]
    print_name: bool,
    /// Stream socket that starts a session for every connection.
    #[arg(long, default_value = "/run/blessh/session.sock")]
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
    #[arg(long, default_value = "/etc/ssh/ssh_host_ed25519_key.pub")]
    host_key: PathBuf,
    /// Most sessions at a time over all transports (the session socket's
    /// MaxConnections should allow as many).
    #[arg(long, default_value_t = 4)]
    max_sessions: usize,
    /// Do not offer an L2CAP channel: clients use the GATT stream only.
    #[arg(long)]
    no_l2cap: bool,
    /// Receive MTU of the L2CAP channel, in bytes.
    #[arg(long, default_value_t = 1024)]
    l2cap_mtu: u16,
    /// Receive MTU of a second L2CAP channel that clients try when the first
    /// one stalls (0: no second channel). Small SDUs avoid flow control
    /// stalls between some stacks on controllers without LE Data Length
    /// Extension.
    #[arg(long, default_value_t = 64)]
    l2cap_small_mtu: u16,
    /// Seconds a client has for its channel hello.
    #[arg(long, default_value_t = 10)]
    hello_timeout: u64,
    /// Advertise as discoverable, so that the server also appears in the
    /// Bluetooth device lists of phones and computers; blessh clients find
    /// it either way.
    #[arg(long)]
    discoverable: bool,
}

/// Sessions in progress over both transports, per client device.
#[derive(Default)]
struct Sessions {
    per_device: HashMap<Address, usize>,
    /// L2CAP channels open per device, sessions or not (yet).
    channels: HashMap<Address, usize>,
    /// When a device last did something that shows it still uses its
    /// link: a channel closed, or it read INFO, which clients do before
    /// they open a channel.
    last_activity: HashMap<Address, Instant>,
}

type Shared = Arc<Mutex<Sessions>>;

impl Sessions {
    fn total(&self) -> usize {
        self.per_device.values().sum()
    }

    fn has(&self, addr: Address) -> bool {
        self.per_device.contains_key(&addr)
    }

    fn devices(&self) -> Vec<Address> {
        self.per_device.keys().copied().collect()
    }
}

/// Counts an L2CAP channel from accept to close. When the last channel of
/// a device closes, the link goes down after a grace period unless another
/// channel opened or the device read INFO meanwhile (a client reads it
/// before it opens a channel): the client may open the next one right away,
/// and controllers that do not advertise while connected become visible
/// again. The grace period belongs to the link: if the client ends it
/// first, the timer stops, so that it cannot cut the client's next link
/// (phones keep their address across links).
struct ChannelGuard {
    sessions: Shared,
    addr: Address,
    adapter: Adapter,
    shutdown: watch::Receiver<bool>,
}

impl ChannelGuard {
    fn new(
        sessions: &Shared,
        addr: Address,
        adapter: &Adapter,
        shutdown: &watch::Receiver<bool>,
    ) -> Self {
        *sessions.lock().unwrap().channels.entry(addr).or_default() += 1;
        Self {
            sessions: sessions.clone(),
            addr,
            adapter: adapter.clone(),
            shutdown: shutdown.clone(),
        }
    }
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        let addr = self.addr;
        let idle = {
            let mut s = self.sessions.lock().unwrap();
            let n = s.channels.entry(addr).or_default();
            *n = n.saturating_sub(1);
            let idle = *n == 0;
            if idle {
                s.channels.remove(&addr);
            }
            s.last_activity.insert(addr, Instant::now());
            idle
        };
        if !idle || *self.shutdown.borrow() {
            return;
        }
        let (sessions, adapter) = (self.sessions.clone(), self.adapter.clone());
        tokio::spawn(async move {
            if tokio::time::timeout(LINK_GRACE, disconnected(&adapter, addr))
                .await
                .is_ok()
            {
                return;
            }
            let s = sessions.lock().unwrap();
            let quiet = s
                .last_activity
                .get(&addr)
                .is_some_and(|t| t.elapsed() >= LINK_GRACE);
            if quiet && !s.channels.contains_key(&addr) && !s.has(addr) {
                drop(s);
                info!(
                    "{addr}: no channel for {}s, disconnecting",
                    LINK_GRACE.as_secs()
                );
                disconnect(&adapter, addr);
            }
        });
    }
}

/// Counts a session for as long as it exists, however it ends.
struct Counted {
    sessions: Shared,
    addr: Address,
}

impl Counted {
    /// Admits a session unless `max` are already in progress.
    fn admit(sessions: &Shared, addr: Address, max: usize) -> Option<Self> {
        let mut s = sessions.lock().unwrap();
        if s.total() >= max {
            return None;
        }
        *s.per_device.entry(addr).or_default() += 1;
        Some(Self {
            sessions: sessions.clone(),
            addr,
        })
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        let mut s = self.sessions.lock().unwrap();
        if let Some(n) = s.per_device.get_mut(&self.addr) {
            *n -= 1;
            if *n == 0 {
                s.per_device.remove(&self.addr);
            }
        }
    }
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

pub fn main(argv: Vec<String>) -> i32 {
    logging::init("info");
    let args = Args::parse_from(argv);
    let name = match server_name(args.name.as_deref()) {
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
    // Tasks blocked on reads must not hold the exit up.
    rt.shutdown_background();
    code
}

/// The name to advertise: `--name`, else the static host name, else the
/// kernel's (possibly transient) one.
pub(crate) fn server_name(given: Option<&str>) -> Result<String, String> {
    if let Some(n) = given {
        name::check(n).map_err(|e| format!("--name {n:?}: {e}"))?;
        return Ok(n.to_owned());
    }
    ["/etc/hostname", "/proc/sys/kernel/hostname"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .find_map(|s| {
            s.lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .and_then(name::derive)
        })
        .ok_or_else(|| "no host name to advertise; set one with --name".into())
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
        "it is blocked by rfkill; unblock it with `sudo rfkill unblock bluetooth` \
         (on a desktop, its Bluetooth settings may block it again at login)"
            .into()
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
/// How long a stopping daemon waits for the active session to send CLOSE.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

/// How a session ended, for the main loop.
struct SessionEnd {
    /// Disconnect the device after a grace period.
    disconnect: bool,
    /// Notification socket to keep for a reconnect over the same link.
    notify: Option<Arc<CharacteristicWriter>>,
}

/// How long a link stays up after its last L2CAP session, for a client
/// that opens the next channel right away. Clients typically come back
/// within ten seconds or so; a grace period of that length met them while
/// the disconnect, which takes up to three seconds to reach the radio on a
/// Pi Zero 2 W, was under way, and cut their new session.
const LINK_GRACE: Duration = Duration::from_secs(30);

const ADV_INTERVAL: Duration = Duration::from_millis(400);

/// Advertises the service and the name. Without `discoverable` the
/// advertisement lacks the discoverable flags, which keeps the server out
/// of the device lists of system settings; clients scan for the service
/// UUID and find it anyway (protocol.md, "Discovery").
async fn advertise(
    adapter: &Adapter,
    name: &str,
    discoverable: bool,
) -> bluer::Result<AdvertisementHandle> {
    let adv = Advertisement {
        service_uuids: [SERVICE_UUID].into_iter().collect(),
        discoverable: Some(discoverable),
        local_name: Some(name.to_owned()),
        // BlueZ defaults to 1.28 s; a phone can only start connecting after it
        // hears an advertisement, so a shorter interval makes connects faster.
        min_interval: Some(ADV_INTERVAL),
        max_interval: Some(ADV_INTERVAL),
        ..Default::default()
    };
    adapter.advertise(adv).await
}

/// How long the adapter's GATT and advertising interfaces may take to
/// appear after the adapter is powered (they do a moment later when
/// bluetoothd has just started).
const INTERFACES_TIMEOUT: Duration = Duration::from_secs(30);

async fn wait_interfaces(adapter: &Adapter) -> bluer::Result<()> {
    let deadline = Instant::now() + INTERFACES_TIMEOUT;
    loop {
        match adapter.active_advertising_instances().await {
            Ok(_) => return Ok(()),
            Err(e) if Instant::now() < deadline => {
                debug!("advertising interface not up yet: {e}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn serve(args: Args, name: String) -> Result<(), Box<dyn std::error::Error>> {
    let session = bluer::Session::new().await?;
    let adapter = wait_adapter(&session, args.adapter.as_deref()).await?;
    wait_powered(&adapter).await?;
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

    let sessions: Shared = Arc::default();
    // The L2CAP channel is the main transport; the GATT stream stays for
    // clients that cannot open one.
    // Channels in the order clients try them.
    let mut listeners: Vec<(StreamListener, u16, u16)> = Vec::new();
    if !args.no_l2cap {
        let mtus = [args.l2cap_mtu, args.l2cap_small_mtu];
        for mtu in mtus.into_iter().filter(|m| *m > 0) {
            match bluez::l2cap_listen(adapter.address().await?, mtu, 4).await {
                Ok(l) => {
                    let psm = l.as_ref().local_addr()?.psm;
                    listeners.push((l, psm, mtu));
                }
                Err(e) => warn!("cannot listen for L2CAP channels with MTU {mtu}: {e}"),
            }
        }
        if listeners.is_empty() {
            warn!("no L2CAP channel; serving the GATT stream only");
        }
    }
    let psms: Vec<u16> = listeners.iter().map(|(_, p, _)| *p).collect();
    let psm = psms.first().copied();

    let info_name = name.clone();
    let info_psms = psms.clone();
    // The device whose stream is open or being opened. INFO tells other
    // devices that the server is busy before they subscribe: BlueZ closes a
    // notification socket of another device when a subscribed device
    // disconnects (5.82 to at least 5.87), and before 5.69 all devices share
    // one write socket, so a turned-away client must not get that far.
    let holder: Arc<std::sync::Mutex<Option<Address>>> = Arc::default();
    let (info_holder, info_adapter, info_sessions) =
        (holder.clone(), adapter.clone(), sessions.clone());
    let (rx_control, rx_handle) = characteristic_control();
    let (tx_control, tx_handle) = characteristic_control();
    let mut characteristics = vec![
        Characteristic {
            uuid: INFO_UUID,
            read: Some(CharacteristicRead {
                read: true,
                fun: Box::new(move |req| {
                    // A client reconnecting right after its previous session
                    // reads INFO before it opens a channel: keep the link
                    // from being dropped under it by the grace timer.
                    debug!("{}: reads INFO", req.device_address);
                    info_sessions
                        .lock()
                        .unwrap()
                        .last_activity
                        .insert(req.device_address, Instant::now());
                    let holder = *info_holder.lock().unwrap();
                    let (adapter, name, psms) =
                        (info_adapter.clone(), info_name.clone(), info_psms.clone());
                    async move {
                        let busy = match holder {
                            Some(h) if h != req.device_address => is_connected(&adapter, h).await,
                            _ => false,
                        };
                        let v = protocol::info(&name, req.mtu, busy, &psms).into_bytes();
                        protocol::read_at(&v, req.offset)
                            .map(<[u8]>::to_vec)
                            .ok_or(ReqError::InvalidOffset)
                    }
                    .boxed()
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
    ];
    if let Some(psm) = psm {
        // Apple's PSM characteristic: iOS and macOS clients read it.
        characteristics.push(Characteristic {
            uuid: PSM_UUID,
            read: Some(CharacteristicRead {
                read: true,
                fun: Box::new(move |req| {
                    async move {
                        protocol::read_at(&psm.to_le_bytes(), req.offset)
                            .map(<[u8]>::to_vec)
                            .ok_or(ReqError::InvalidOffset)
                    }
                    .boxed()
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    let app = Application {
        services: vec![Service {
            uuid: SERVICE_UUID,
            primary: true,
            characteristics,
            ..Default::default()
        }],
        ..Default::default()
    };
    // BlueZ drops the registrations when the adapter goes away (unplugged,
    // reset by the USB stack) and does not restore them when it comes back.
    let adapter_events = session.events().await?;
    // Nor does it restore an advertisement in the controller after the
    // adapter was switched off and on (bluetoothctl, rfkill, a desktop's
    // Bluetooth switch) when the controller uses extended advertising: the
    // advertisement stays registered but is not on the air. bluer ends this
    // stream when bluetoothd removes any interface of the adapter, which it
    // does around a power change, so an end counts as one too.
    let mut power_events: futures::stream::BoxStream<'static, AdapterEvent> =
        adapter.events().await?.boxed();
    wait_interfaces(&adapter).await?;
    let _app = adapter.serve_gatt_application(app).await?;
    let mut adv = Some(advertise(&adapter, &name, args.discoverable).await?);
    let channels: Vec<String> = listeners
        .iter()
        .map(|(_, p, m)| format!("PSM 0x{p:02x} (MTU {m})"))
        .collect();
    info!(
        "ready (BlueZ GATT application and advertisement registered{})",
        if channels.is_empty() {
            String::new()
        } else {
            format!(", L2CAP channels on {}", channels.join(", "))
        }
    );

    // Tells the sessions that the daemon is stopping.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let accept_tasks: Vec<_> = listeners
        .into_iter()
        .map(|(l, _, _)| {
            let (adapter, args, sessions, shutdown) = (
                adapter.clone(),
                args.clone(),
                sessions.clone(),
                shutdown_rx.clone(),
            );
            tokio::spawn(accept_loop(l, adapter, args, sessions, shutdown))
        })
        .collect();
    tokio::pin!(rx_control, tx_control, adapter_events);
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
    // Cancels that disconnect when the link goes down by itself first: a
    // client that disconnects and connects again for the next session must
    // not lose its new link, which the timer cannot tell from the old one.
    let (left_tx, mut left_rx) = mpsc::unbounded_channel::<Address>();
    let mut left_watch: Option<JoinHandle<()>> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        *holder.lock().unwrap() = active.as_ref().map(|(a, _)| *a).or(pending.address());
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            Some(ev) = adapter_events.next() => {
                if matches!(&ev, SessionEvent::AdapterRemoved(n) if n == adapter.name()) {
                    // Exit with an error: systemd restarts the daemon, which
                    // waits for the adapter and registers again.
                    return Err(format!("Bluetooth adapter {} was removed", adapter.name()).into());
                }
            }
            ev = power_events.next() => match ev {
                Some(AdapterEvent::PropertyChanged(AdapterProperty::Powered(on))) => {
                    // Registered again on a tick once the adapter is on.
                    adv = None;
                    if on {
                        info!("adapter {} is on again, advertising again", adapter.name());
                    } else {
                        warn!("adapter {} was switched off; waiting for it", adapter.name());
                    }
                }
                Some(_) => {}
                None => {
                    debug!("adapter {} changed its interfaces; advertising again", adapter.name());
                    adv = None;
                    match adapter.events().await {
                        Ok(s) => power_events = s.boxed(),
                        Err(e) => {
                            warn!("cannot follow adapter {}: {e}", adapter.name());
                            power_events = futures::stream::pending().boxed();
                        }
                    }
                }
            },
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
            Some(addr) = left_rx.recv() => {
                if deferred_disconnect.is_some_and(|(a, _)| a == addr) {
                    debug!("{addr}: link went down after the session, no disconnect needed");
                    deferred_disconnect = None;
                }
            }
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
                    if let Some(w) = left_watch.take() {
                        w.abort();
                    }
                    let (adapter, left) = (adapter.clone(), left_tx.clone());
                    left_watch = Some(tokio::spawn(async move {
                        disconnected(&adapter, addr).await;
                        let _ = left.send(addr);
                    }));
                }
                // The kernel resumes advertising on disconnect; re-registering
                // here raced with reconnecting clients in tests, so it is opt-in.
                if args.readvertise {
                    adv = None;
                    match advertise(&adapter, &name, args.discoverable).await {
                        Ok(h) => adv = Some(h),
                        Err(e) => error!("re-registering advertisement failed: {e}"),
                    }
                }
            }
            _ = tick.tick() => {
                if adv.is_none() && adapter.is_powered().await.unwrap_or(false) {
                    match advertise(&adapter, &name, args.discoverable).await {
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
            let Some(counted) = Counted::admit(&sessions, addr, args.max_sessions) else {
                info!(
                    "{addr}: rejecting, {} sessions in progress",
                    args.max_sessions
                );
                let busy = Frame::Close {
                    reason: CloseReason::Busy,
                    message: "no free session".into(),
                };
                let _ = writer.try_send(&busy.encode(0));
                disconnect(&adapter, addr);
                continue;
            };
            let adapter = adapter.clone();
            let args = args.clone();
            let shutdown = shutdown_rx.clone();
            active = Some((
                addr,
                tokio::spawn(async move {
                    run_session(adapter, addr, reader, writer, args, counted, shutdown).await
                }),
            ));
        }
    }

    info!("shutting down");
    for t in &accept_tasks {
        t.abort();
    }
    let others = sessions.lock().unwrap().devices();
    // The GATT session sends CLOSE, so the client learns why it ended
    // instead of keeping an idle link to a server that is gone; L2CAP
    // sessions end with their channel.
    let _ = shutdown_tx.send(true);
    if let Some((addr, mut task)) = active {
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
        disconnect_now(&adapter, addr).await;
    }
    for addr in others {
        disconnect_now(&adapter, addr).await;
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

/// Ends the link of a stopping daemon: gives the client a moment to read
/// CLOSE and disconnect by itself, then disconnects it.
async fn disconnect_now(adapter: &Adapter, addr: Address) {
    if tokio::time::timeout(SHUTDOWN_GRACE, disconnected(adapter, addr))
        .await
        .is_ok()
    {
        return;
    }
    if let Ok(dev) = adapter.device(addr) {
        match tokio::time::timeout(Duration::from_secs(2), dev.disconnect()).await {
            Ok(Err(e)) => debug!("{addr}: disconnect failed: {e}"),
            Err(_) => debug!("{addr}: disconnect timed out"),
            Ok(Ok(())) => {}
        }
    }
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
/// way it ended when it is over; blessh-session logs the same id in the
/// journal of the sshd it starts, which ties logins to BLE devices.
async fn run_session(
    adapter: Adapter,
    addr: Address,
    reader: CharacteristicReader,
    writer: Arc<CharacteristicWriter>,
    args: Args,
    counted: Counted,
    mut shutdown: watch::Receiver<bool>,
) -> SessionEnd {
    let mut tx = GattTx(writer.clone());
    let mut link = Link::new(writer.mtu(), args.window);
    let started = Instant::now();
    let preamble = Preamble {
        id: session::new_id(),
        device: addr.to_string(),
        address_type: address_type(&adapter, addr).await.into(),
        transport: "gatt".into(),
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
        "{addr}: session {id}: client connected (gatt, {} address, notify MTU {}, write MTU {})",
        preamble.address_type,
        writer.mtu(),
        reader.mtu()
    );
    let mut reader = GattRx(reader);

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
        let stopping = async {
            let _ = shutdown.wait_for(|s| *s).await;
        };
        let gone = async {
            tokio::select! {
                _ = gone => {}
                _ = stopping => {}
            }
        };
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
        Ok(Some(Outcome::Cancelled)) if *shutdown.borrow() => {
            link.close(CloseReason::SessionUnavailable, "server is shutting down");
            while let Some(p) = link.poll_transmit() {
                if tx.send(&p).await.is_err() {
                    break;
                }
            }
            ("server is shutting down".into(), false)
        }
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
    drop(counted);
    SessionEnd {
        disconnect: !lost,
        notify: clean.then_some(writer),
    }
}

/// Accepts L2CAP channels and runs a session on each.
async fn accept_loop(
    listener: StreamListener,
    adapter: Adapter,
    args: Args,
    sessions: Shared,
    shutdown: watch::Receiver<bool>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let guard = ChannelGuard::new(&sessions, peer.addr, &adapter, &shutdown);
                let (args, sessions, shutdown) = (args.clone(), sessions.clone(), shutdown.clone());
                tokio::spawn(async move {
                    run_l2cap_session(stream, peer, args, sessions, shutdown).await;
                    drop(guard);
                });
            }
            Err(e) => {
                error!("accepting an L2CAP channel failed: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Refuses a session on an L2CAP channel in the handshake reply, then
/// closes the channel.
async fn refuse(mut stream: Stream, hello: &channel::Header, reason: &str) {
    let _ = channel::refuse(&mut stream, hello, reason).await;
    let _ = stream.shutdown().await;
}

/// Runs one session over an L2CAP channel: raw SSH between the channel and
/// the session socket. Logged like a GATT session, with the same id in the
/// session unit's journal.
async fn run_l2cap_session(
    mut stream: Stream,
    peer: SocketAddr,
    args: Args,
    sessions: Shared,
    mut shutdown: watch::Receiver<bool>,
) {
    let addr = peer.addr;
    let hello = match tokio::time::timeout(
        Duration::from_secs(args.hello_timeout),
        channel::read_hello(&mut stream),
    )
    .await
    {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => {
            info!("{addr}: dropping an L2CAP channel without a blessh hello: {e}");
            return;
        }
        Err(_) => {
            info!("{addr}: dropping an L2CAP channel: no hello in time");
            return;
        }
    };
    let address_type = match peer.addr_type {
        AddressType::LePublic => "public",
        AddressType::LeRandom => "random",
        _ => "unknown",
    };
    let id = session::new_id();
    let started = Instant::now();
    let mtu = stream.as_ref().send_mtu().unwrap_or(0);
    info!("{addr}: session {id}: client connected (l2cap, {address_type} address, MTU {mtu})");
    let Some(counted) = Counted::admit(&sessions, addr, args.max_sessions) else {
        info!(
            "{addr}: session {id}: refused, {} sessions in progress",
            args.max_sessions
        );
        refuse(
            stream,
            &hello,
            &format!("busy ({} sessions)", args.max_sessions),
        )
        .await;
        return;
    };
    let preamble = Preamble {
        id: id.clone(),
        device: addr.to_string(),
        address_type: address_type.into(),
        transport: "l2cap".into(),
    };
    let local = async {
        let mut s = UnixStream::connect(&args.session_socket).await?;
        s.write_all(preamble.encode().as_bytes()).await?;
        Ok::<_, io::Error>(s)
    }
    .await;
    let local = match local {
        Ok(s) => s,
        Err(e) => {
            error!(
                "{addr}: session {id}: cannot start session via {}: {e}",
                args.session_socket.display()
            );
            refuse(stream, &hello, "session unavailable").await;
            drop(counted);
            return;
        }
    };
    if let Err(e) = channel::accept(&mut stream, &hello).await {
        info!("{addr}: session {id}: channel handshake failed: {e}");
        return;
    }
    info!("{addr}: session {id}: SSH session started");
    let stopping = async {
        let _ = shutdown.wait_for(|s| *s).await;
    };
    let result = relay::run(stream, local, bluez::is_link_end, stopping).await;
    let (end, from_client, to_client) = match &result {
        Ok(o) => (
            match &o.end {
                End::Local => "server ended the session".to_owned(),
                End::Peer(None) => "client closed the channel".to_owned(),
                End::Peer(Some(e)) => {
                    format!("client disconnected: {}", bluez::link_end_reason(e))
                }
                End::Cancelled => "server is shutting down".to_owned(),
            },
            o.from_peer,
            o.to_peer,
        ),
        Err(e) => (format!("channel error: {e}"), 0, 0),
    };
    info!(
        "{addr}: session {id}: ended after {}s, {from_client} bytes from client, {to_client} bytes to client: {end}",
        started.elapsed().as_secs()
    );
    drop(counted);
}
