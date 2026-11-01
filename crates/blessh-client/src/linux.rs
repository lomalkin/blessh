//! Linux client: bluetoothd (through bluer) scans for the server; the LE
//! link is then made through the kernel by this process, which reads INFO
//! over its own ATT channel and runs the session over an L2CAP channel.
//! bluetoothd's GATT client is used only for the GATT stream (`--gatt`, or
//! a server without an L2CAP channel) and for a device that is already
//! connected through bluetoothd.
//!
//! The direct link matters: bluetoothd connects a dual-mode device such as
//! a Mac over BR/EDR, and on any LE link it discovers the peer's services
//! and probes them with its profiles, which on a server that exposes a
//! service needing encryption (BLE MIDI from PipeWire) ends in a pairing
//! request and a lost link. On a link whose ATT channel this process holds,
//! bluetoothd does neither.

use std::{collections::HashSet, error::Error, future::Future, sync::Arc, time::Duration};

use blessh_ble::bluez::{self, AttLink, GattRx, GattTx};
use blessh_core::{
    bridge::{self, Outcome, PacketTx},
    channel,
    link::Link,
    name,
    protocol::{self, printable, CloseReason, INFO_UUID, RX_UUID, SERVICE_UUID, TX_UUID},
    relay::{self, End},
};
use bluer::{
    gatt::{
        remote::{Characteristic, CharacteristicWriteRequest},
        WriteOp,
    },
    Adapter, AdapterEvent, Address, Device, DiscoveryFilter, DiscoveryTransport,
};
use futures::StreamExt;
use log::{debug, info, warn};

use crate::cli::{stop_signal, ProxyArgs, ScanArgs};

/// How long making the LE link may take (the device must be advertising).
const LINK_TIMEOUT: Duration = Duration::from_secs(20);
/// How long one ATT request may take.
const ATT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long opening the L2CAP channel may take, link setup included.
const L2CAP_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long bluetoothd may take to discover the server's services.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long another blessh may take to learn the PSM of a server it is
/// connecting to.
const PSM_WAIT: Duration = Duration::from_secs(5);
/// How long the scan matches names only before it asks servers whose
/// name does not match for their INFO.
const PROBE_AFTER: Duration = Duration::from_secs(4);
/// How long `--keep-link` keeps the link up after the session.
const KEEP_LINK: Duration = Duration::from_secs(5);

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn scan(args: ScanArgs) -> i32 {
    run(scan_async(args))
}

pub fn proxy(args: ProxyArgs) -> i32 {
    run(proxy_async(args))
}

fn run(f: impl Future<Output = Result<i32>>) -> i32 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match rt.block_on(f) {
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

async fn adapter(name: Option<&str>) -> Result<Adapter> {
    let session = bluer::Session::new().await?;
    let adapter = match name {
        Some(n) => session.adapter(n)?,
        None => session.default_adapter().await?,
    };
    if !adapter.is_powered().await? {
        return Err(format!("adapter {} is powered off", adapter.name()).into());
    }
    Ok(adapter)
}

async fn scan_async(args: ScanArgs) -> Result<i32> {
    let adapter = adapter(args.adapter.as_deref()).await?;
    let local = adapter.address().await?;
    find(&adapter, local, None, args.timeout, true).await?;
    Ok(0)
}

/// Writes each frame with response through D-Bus.
struct RequestTx(Characteristic);

impl PacketTx for RequestTx {
    async fn send(&mut self, pkt: &[u8]) -> std::io::Result<()> {
        let req = CharacteristicWriteRequest {
            op_type: WriteOp::Request,
            ..Default::default()
        };
        self.0
            .write_ext(pkt, &req)
            .await
            .map_err(std::io::Error::other)
    }
}

/// A server the scan found.
struct Found {
    dev: Device,
    /// The link and INFO when the server was identified by the name in its
    /// INFO rather than by the name bluetoothd shows.
    link: Option<(AttLink, String)>,
}

/// How many times making the LE link is tried: connection setup fails now
/// and then over the air (HCI status 0x3e, which the kernel reports as
/// ENOSYS) or times out.
const LINK_ATTEMPTS: u32 = 3;

/// Makes the LE link and reads INFO.
async fn probe(local: Address, dev: &Device) -> Result<(AttLink, String)> {
    let peer_type = dev.address_type().await?;
    let mut attempt = 1;
    let link = loop {
        match AttLink::connect(local, dev.address(), peer_type, LINK_TIMEOUT).await {
            Ok(l) => break l,
            Err(e) if attempt < LINK_ATTEMPTS && e.raw_os_error() != Some(libc::EBUSY) => {
                debug!("{}: link setup failed: {e}; retrying", dev.address());
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };
    let info = link
        .read_info(ATT_TIMEOUT)
        .await
        .map_err(|e| format!("cannot read the server's INFO: {e}"))?;
    Ok((link, info))
}

/// Finds the server: by address or advertised name, or the first one seen
/// when `target` is None. With `list`, prints every server instead.
///
/// bluetoothd may show a device under another name than the one it
/// advertises (its GAP device name, read once over some connection), so a
/// server whose name does not match is asked for its INFO.
async fn find(
    adapter: &Adapter,
    local: Address,
    target: Option<&str>,
    timeout: u64,
    list: bool,
) -> Result<Option<Found>> {
    if let Some(addr) = target.and_then(|t| t.parse::<Address>().ok()) {
        if let Ok(dev) = adapter.device(addr) {
            if dev.is_connected().await.unwrap_or(false)
                || dev.uuids().await.ok().flatten().is_some()
            {
                return Ok(Some(Found { dev, link: None }));
            }
        }
    }
    // Without the service UUID as a filter: with it, BlueZ may not record
    // the name a server sends in its scan response. Devices are matched by
    // their advertised UUIDs below.
    if let Err(e) = adapter
        .set_discovery_filter(DiscoveryFilter {
            transport: DiscoveryTransport::Le,
            ..Default::default()
        })
        .await
    {
        debug!("cannot set the discovery filter: {e}");
    }
    // bluetoothd may not answer StartDiscovery while another process
    // connects to a device; the devices it knows are checked meanwhile.
    let mut events = start_discovery(adapter).await?;
    let deadline = tokio::time::sleep(Duration::from_secs(timeout));
    tokio::pin!(deadline);
    // Advertised UUIDs and names can arrive after DeviceAdded, so devices
    // are re-checked until they match or the scan ends.
    // Devices BlueZ already knows produce no DeviceAdded event.
    let mut candidates: Vec<Address> = adapter.device_addresses().await?;
    let mut listed: HashSet<Address> = HashSet::new();
    let mut probed: HashSet<Address> = HashSet::new();
    // Servers in range with their names, for the error message.
    let mut seen: Vec<(Address, String)> = Vec::new();
    let probe_from = tokio::time::Instant::now() + PROBE_AFTER;
    let mut recheck = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = &mut deadline => {
                if let (Some(t), false) = (target, list) {
                    return Err(not_found(t, &seen).into());
                }
                return Ok(None);
            }
            ev = async { events.as_mut()?.next().await }, if events.is_some() => {
                if let Some(AdapterEvent::DeviceAdded(addr)) = ev {
                    if !candidates.contains(&addr) {
                        candidates.push(addr);
                    }
                }
            }
            _ = recheck.tick() => {
                if events.is_none() {
                    events = start_discovery(adapter).await?;
                }
                for addr in adapter.device_addresses().await? {
                    if !candidates.contains(&addr) {
                        candidates.push(addr);
                    }
                }
            }
        }
        // Names first: a server is asked for its INFO only when none
        // matches by name for a while.
        let probing = tokio::time::Instant::now() >= probe_from;
        for addr in candidates.clone() {
            let dev = adapter.device(addr)?;
            let target_addr = target.is_some_and(|t| t.eq_ignore_ascii_case(&addr.to_string()));
            let advertises = dev
                .uuids()
                .await
                .ok()
                .flatten()
                .unwrap_or_default()
                .contains(&SERVICE_UUID);
            let name = dev.name().await.ok().flatten().unwrap_or_default();
            let alias = dev.alias().await.unwrap_or_default();
            // bluetoothd forgets the UUIDs a device advertised when a link
            // to it drops, until it hears it again, and a server may not
            // advertise while connected: a device seen in this scan or
            // connected whose name matches is a candidate all the same, and
            // its INFO tells.
            let named = !list
                && target.is_some_and(|t| name::same(t, &name) || name::same(t, &alias))
                && (dev.rssi().await.ok().flatten().is_some()
                    || dev.is_connected().await.unwrap_or(false));
            if !advertises && !target_addr && !named {
                continue;
            }
            if !seen.iter().any(|(a, _)| *a == addr) {
                seen.push((addr, name.clone()));
            }
            if list {
                if listed.insert(addr) {
                    println!(
                        "{addr}  {name}  rssi={}",
                        dev.rssi().await.ok().flatten().unwrap_or_default()
                    );
                }
                continue;
            }
            match target {
                None => return Ok(Some(Found { dev, link: None })),
                Some(t) if target_addr || name::same(t, &name) || name::same(t, &alias) => {
                    return Ok(Some(Found { dev, link: None }))
                }
                Some(t) if probing && probed.insert(addr) => match probe(local, &dev).await {
                    Ok((link, info))
                        if protocol::info_value(&info, "name")
                            .is_some_and(|n| name::same(n, t)) =>
                    {
                        return Ok(Some(Found {
                            dev,
                            link: Some((link, info)),
                        }))
                    }
                    Ok((link, info)) => {
                        release(&dev, link, !others_use(addr)).await;
                        let real = protocol::info_value(&info, "name").unwrap_or_default();
                        debug!("{addr} ({name:?}) is {real:?}, not {t:?}");
                        if let Some((_, n)) = seen.iter_mut().find(|(a, _)| *a == addr) {
                            *n = real.to_owned();
                        }
                    }
                    Err(e) => debug!("{addr} ({name:?}): {e}"),
                },
                _ => {}
            }
        }
    }
}

type Discovery = std::pin::Pin<Box<dyn futures::Stream<Item = AdapterEvent> + Send>>;

/// How long bluetoothd may take to start discovery.
const DISCOVERY_START_TIMEOUT: Duration = Duration::from_secs(2);

/// Starts discovery; None if bluetoothd does not (yet), for example while
/// another process connects to a device. The caller tries again later.
async fn start_discovery(adapter: &Adapter) -> bluer::Result<Option<Discovery>> {
    match tokio::time::timeout(DISCOVERY_START_TIMEOUT, adapter.discover_devices()).await {
        Ok(Ok(events)) => Ok(Some(Box::pin(events))),
        Ok(Err(e))
            if matches!(
                e.kind,
                bluer::ErrorKind::InProgress | bluer::ErrorKind::NotReady
            ) =>
        {
            debug!("bluetoothd did not start discovery: {e}");
            Ok(None)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            debug!("bluetoothd did not start discovery in time");
            Ok(None)
        }
    }
}

/// Why no server matched `target`, naming those that were in range.
fn not_found(target: &str, seen: &[(Address, String)]) -> String {
    let seen: Vec<&str> = seen
        .iter()
        .map(|(_, n)| n.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    if seen.is_empty() {
        format!("no blessh server named {target:?} found")
    } else {
        format!(
            "no blessh server named {target:?} found; servers in range: {}",
            seen.join(", ")
        )
    }
}

struct Chars {
    info: Option<Characteristic>,
    rx: Characteristic,
    tx: Characteristic,
}

async fn characteristics(dev: &Device) -> bluer::Result<Option<Chars>> {
    for service in dev.services().await? {
        if service.uuid().await? != SERVICE_UUID {
            continue;
        }
        let (mut info, mut rx, mut tx) = (None, None, None);
        for c in service.characteristics().await? {
            match c.uuid().await? {
                u if u == INFO_UUID => info = Some(c),
                u if u == RX_UUID => rx = Some(c),
                u if u == TX_UUID => tx = Some(c),
                _ => {}
            }
        }
        if let (Some(rx), Some(tx)) = (rx, tx) {
            return Ok(Some(Chars { info, rx, tx }));
        }
    }
    Ok(None)
}

/// `Channels` from the `[GATT]` section of BlueZ's main.conf, if set.
fn eatt_channels(conf: &str) -> Option<u8> {
    let mut section = "";
    let mut channels = None;
    for line in conf.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s;
        } else if let Some((k, v)) = line.split_once('=') {
            if section == "GATT" && k.trim() == "Channels" {
                channels = v.trim().parse().ok();
            }
        }
    }
    channels
}

/// Why this machine's BlueZ may open several ATT bearers (EATT), which the
/// GATT stream does not support. BlueZ before 5.67 does so by default.
fn eatt_hint() -> Option<String> {
    // EATT needs the kernel's ECRED L2CAP mode; the server package turns it
    // off on BlueZ before 5.67 (docs/linux.md, "Bookworm").
    let ecred = std::fs::read_to_string("/sys/module/bluetooth/parameters/enable_ecred");
    if ecred.is_ok_and(|v| v.trim() == "N") {
        return None;
    }
    let conf = std::fs::read_to_string("/etc/bluetooth/main.conf").unwrap_or_default();
    match eatt_channels(&conf) {
        Some(n) if n > 1 => return Some(format!("Channels = {n} in /etc/bluetooth/main.conf")),
        Some(_) => return None,
        None => {}
    }
    let out = std::process::Command::new("bluetoothctl")
        .arg("--version")
        .output()
        .ok()?;
    let version = String::from_utf8_lossy(&out.stdout);
    let version = version.trim().rsplit(' ').next()?.to_owned();
    let (major, minor) = version.split_once('.')?;
    let old = (major.parse::<u32>().ok()?, minor.parse::<u32>().ok()?) < (5, 67);
    old.then(|| format!("BlueZ {version} enables EATT by default"))
}

/// What the blessh instances of one user share about a device: a lock,
/// held shared while connected, so that the last one to leave can tell it
/// is the last (and disconnect a link bluetoothd holds), and the server's
/// PSM, for an instance that finds the ATT channel taken by another one and
/// opens its channel over the same link.
struct DeviceLock(Option<std::fs::File>);

impl DeviceLock {
    fn path(addr: Address) -> Option<std::path::PathBuf> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Some(dir.join(format!("blessh-{}.lock", addr.to_string().replace(':', ""))))
    }

    async fn shared(addr: Address) -> Self {
        use std::os::fd::AsRawFd;
        let Some(path) = Self::path(addr) else {
            return Self(None);
        };
        let file = match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                debug!("cannot open lock file {}: {e}", path.display());
                return Self(None);
            }
        };
        for attempt in 0..100 {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
                return Self(Some(file));
            }
            if attempt == 0 {
                info!("waiting for another blessh to release {addr}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        warn!("another blessh still uses {addr}, connecting anyway");
        Self(None)
    }

    /// True when no other blessh uses the device any more.
    /// Called on the way out only: a failed conversion to the exclusive
    /// lock drops the shared one as well (flock(2)). The last one clears
    /// the PSM, which is valid only while a link is up.
    fn last_user(&self) -> bool {
        use std::os::fd::AsRawFd;
        match &self.0 {
            Some(f) => {
                let last =
                    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) == 0;
                if last {
                    let _ = f.set_len(0);
                }
                last
            }
            None => true,
        }
    }

    /// Takes the shared lock again after `last_user`, if nobody holds the
    /// exclusive one.
    fn reshare(&self) {
        use std::os::fd::AsRawFd;
        if let Some(f) = &self.0 {
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
        }
    }

    /// The channels another blessh that uses the device now has written.
    fn psms(&self) -> Vec<u16> {
        self.read()
            .map(|s| protocol::info_psms(&s))
            .unwrap_or_default()
    }

    /// Whether another blessh is still making the link.
    fn connecting(&self) -> bool {
        let Some(s) = self.read() else {
            return false;
        };
        let me = std::process::id() as i32;
        s.lines()
            .filter_map(|l| l.strip_prefix("connecting="))
            .filter_map(|p| p.parse::<i32>().ok())
            .any(|pid| pid != me && unsafe { libc::kill(pid, 0) } == 0)
    }

    fn read(&self) -> Option<String> {
        use std::io::{Read, Seek};
        let mut f = self.0.as_ref()?;
        let mut s = String::new();
        f.rewind().ok()?;
        f.read_to_string(&mut s).ok()?;
        Some(s.trim().to_owned())
    }

    fn write(&self, state: &str) {
        use std::io::{Seek, Write};
        if let Some(mut f) = self.0.as_ref() {
            let _ = f.set_len(0);
            let _ = f.rewind();
            let _ = writeln!(f, "{state}");
        }
    }

    fn set_psms(&self, psms: &[u16]) {
        let list: Vec<String> = psms.iter().map(u16::to_string).collect();
        self.write(&format!("psm={};chan={}", list.join(","), channel::VERSION));
    }

    /// Notes that this process is about to make the link; the PSM replaces
    /// the notes once known.
    fn set_connecting(&self) {
        use std::io::{Seek, SeekFrom, Write};
        if let Some(mut f) = self.0.as_ref() {
            if self.psms().is_empty() && f.seek(SeekFrom::End(0)).is_ok() {
                let _ = writeln!(f, "connecting={}", std::process::id());
            }
        }
    }
}

/// Connects through bluetoothd, for the GATT stream.
async fn connect_through_bluetoothd(dev: &Device) -> Result<()> {
    // A link that is still going down (the previous client just left)
    // looks connected, and then drops while bluetoothd discovers the
    // services: connect once more in that case.
    let mut again = true;
    loop {
        if !dev.is_connected().await? {
            let mut attempt = 0;
            loop {
                match dev.connect().await {
                    Ok(()) => break,
                    Err(e) if attempt < 2 => {
                        debug!("connect failed: {e}, retrying");
                        attempt += 1;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        // Connect returns before bluetoothd has discovered the services.
        let deadline = tokio::time::Instant::now() + RESOLVE_TIMEOUT;
        loop {
            if dev.is_services_resolved().await? {
                return Ok(());
            }
            if !dev.is_connected().await? {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                return Err("bluetoothd did not discover the server's services in time".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if !again {
            return Err(
                "the link dropped while bluetoothd discovered the server's services; \
                        it may have found a service that needs encryption and asked the \
                        server to pair (docs/client.md, \"Linux\")"
                    .into(),
            );
        }
        debug!("the link dropped during service discovery, connecting again");
        again = false;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn proxy_async(args: ProxyArgs) -> Result<i32> {
    let adapter = adapter(args.adapter.as_deref()).await?;
    let local = adapter.address().await?;
    let Some(found) = find(
        &adapter,
        local,
        args.target.as_deref(),
        args.scan_timeout,
        false,
    )
    .await?
    else {
        return Err("no blessh server found".into());
    };
    let dev = found.dev.clone();
    let addr = dev.address();
    let lock = DeviceLock::shared(addr).await;
    info!("connecting to {addr}");
    let result = session(local, found, &args, &lock).await;
    if args.keep_link {
        tokio::time::sleep(KEEP_LINK).await;
    } else if lock.last_user() && dev.is_connected().await.unwrap_or(false) {
        let _ = dev.disconnect().await;
    }
    result
}

/// Runs the session over the best transport the server offers.
async fn session(local: Address, found: Found, args: &ProxyArgs, lock: &DeviceLock) -> Result<i32> {
    let Found { dev, link } = found;
    let addr = dev.address();
    let bluez_name = dev.name().await.ok().flatten().unwrap_or_default();
    if args.gatt {
        return bluetoothd_session(local, &dev, args, &bluez_name, None).await;
    }
    let (link, info, shared_psms) = match link {
        Some((link, info)) => (Some(link), Some(info), Vec::new()),
        None => {
            // EBUSY: this machine already has an ATT channel to the server.
            // Another blessh made the link and shares its PSM (or is still
            // making it), or a link that is going down still holds it, or
            // bluetoothd does.
            lock.set_connecting();
            let start = tokio::time::Instant::now();
            loop {
                match probe(local, &dev).await {
                    Ok((link, info)) => break (Some(link), Some(info), Vec::new()),
                    Err(e) if is_busy(e.as_ref()) => {}
                    Err(e) => return Err(format!("{addr}: {e}").into()),
                }
                let p = lock.psms();
                if !p.is_empty() {
                    debug!("{addr}: the link is shared, channels {p:?} from another blessh");
                    break (None, None, p);
                }
                let waited = start.elapsed();
                if waited > PSM_WAIT && !(lock.connecting() && waited < LINK_TIMEOUT + ATT_TIMEOUT)
                {
                    debug!("{addr}: bluetoothd holds the link");
                    return bluetoothd_session(local, &dev, args, &bluez_name, None).await;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    };
    let name = info
        .as_deref()
        .and_then(|i| protocol::info_value(i, "name"))
        .map_or(bluez_name.clone(), str::to_owned);
    if let Some(i) = &info {
        info!("server info: {}", printable(i));
    }
    let psms = match &info {
        Some(i) => protocol::info_psms(i),
        None => shared_psms,
    };
    if !psms.is_empty() {
        lock.set_psms(&psms);
        let peer_type = dev.address_type().await?;
        if let Some(stream) = open_channel(local, addr, peer_type, &psms, args).await? {
            {
                let mtu = stream.as_ref().send_mtu().unwrap_or(0);
                eprintln!("blessh: {name} {addr}: l2cap, mtu {mtu}");
                // The link stays up and the server's bluetoothd keeps
                // getting answers while the ATT channel is served.
                let link = link.map(Arc::new);
                let att = link.clone().map(|l| {
                    tokio::spawn(async move {
                        let e = l.serve().await;
                        debug!("ATT channel ended: {e}");
                    })
                });
                let code = relay_channel(stream).await;
                if args.keep_link {
                    tokio::time::sleep(KEEP_LINK).await;
                }
                if let Some(t) = att {
                    t.abort();
                    let _ = t.await;
                }
                if let Some(l) = link.and_then(Arc::into_inner) {
                    release(&dev, l, !args.keep_link && lock.last_user()).await;
                }
                return code;
            }
        }
        if info.as_deref().is_some_and(|i| !protocol::info_gatt(i)) {
            return Err(
                "no L2CAP channel of the server works, and it serves no GATT stream".into(),
            );
        }
        warn!("no L2CAP channel works; using the GATT stream");
    }
    // The GATT stream needs bluetoothd's GATT client, so the link must be
    // bluetoothd's: give this one up first.
    if let Some(l) = link {
        release(&dev, l, lock.last_user()).await;
        lock.reshare();
    }
    for _ in 0..50 {
        if !dev.is_connected().await? {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bluetoothd_session(local, &dev, args, &name, info).await
}

/// Closes a link this process made. With `disconnect`, the link is taken
/// down before the ATT channel is closed: otherwise bluetoothd takes over
/// the ATT channel of the link that is still up, starts discovering the
/// server's services and, cut off by the disconnect, keeps the device as
/// one without services, ignoring the service UUID it advertises until it
/// restarts.
async fn release(dev: &Device, link: AttLink, disconnect: bool) {
    if disconnect {
        let _ = tokio::time::timeout(Duration::from_secs(3), dev.disconnect()).await;
        for _ in 0..30 {
            if !dev.is_connected().await.unwrap_or(false) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    drop(link);
}

/// Whether another blessh of this user holds the device's lock.
fn others_use(addr: Address) -> bool {
    use std::os::fd::AsRawFd;
    let Some(path) = DeviceLock::path(addr) else {
        return false;
    };
    match std::fs::File::open(path) {
        Ok(f) => (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) != 0,
        Err(_) => false,
    }
}

fn is_busy(e: &(dyn Error + 'static)) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.raw_os_error() == Some(libc::EBUSY))
}

/// A session over a link bluetoothd holds: the L2CAP channel if the
/// server offers one, else the GATT stream.
async fn bluetoothd_session(
    local: Address,
    dev: &Device,
    args: &ProxyArgs,
    name: &str,
    info: Option<String>,
) -> Result<i32> {
    let addr = dev.address();
    connect_through_bluetoothd(dev).await?;
    let Some(chars) = characteristics(dev).await? else {
        return Err("device does not provide the blessh service".into());
    };
    let info = match (info, &chars.info) {
        (Some(i), _) => Some(i),
        (None, Some(c)) => match c.read().await {
            Ok(v) => {
                let v = String::from_utf8_lossy(&v).into_owned();
                info!("server info: {}", printable(&v));
                Some(v)
            }
            Err(e) => {
                debug!("cannot read INFO: {e}");
                None
            }
        },
        (None, None) => None,
    };
    let psms = info.as_deref().map(protocol::info_psms).unwrap_or_default();
    if !psms.is_empty() && !args.gatt {
        let peer_type = dev.address_type().await?;
        if let Some(stream) = open_channel(local, addr, peer_type, &psms, args).await? {
            let mtu = stream.as_ref().send_mtu().unwrap_or(0);
            eprintln!("blessh: {name} {addr}: l2cap, mtu {mtu}");
            return relay_channel(stream).await;
        }
        if info.as_deref().is_some_and(|i| !protocol::info_gatt(i)) {
            return Err(
                "no L2CAP channel of the server works, and it serves no GATT stream".into(),
            );
        }
        warn!("no L2CAP channel works; using the GATT stream");
    }
    gatt_session(&chars, info.as_deref(), args, name, addr).await
}

/// Runs the session over the GATT stream (BleStream).
async fn gatt_session(
    chars: &Chars,
    info: Option<&str>,
    args: &ProxyArgs,
    name: &str,
    addr: Address,
) -> Result<i32> {
    // Subscribing would make things worse for the client that holds the
    // stream (see the server); leave without it.
    if info.is_some_and(protocol::info_busy) {
        return Err("server is busy: another client holds the GATT stream".into());
    }
    if let Some(why) = eatt_hint() {
        warn!(
            "{why}; the GATT stream needs a single ATT bearer: set Channels = 1 in the [GATT] \
             section of /etc/bluetooth/main.conf and restart bluetooth"
        );
    }
    let notify = chars.tx.notify_io().await?;
    let frame = |mtu: usize| args.max_frame.map_or(mtu, |m| m.min(mtu));
    if args.write_with_response {
        // The write socket is not acquired: BlueZ refuses WriteValue while
        // it is held. Both directions share the ATT MTU, which BlueZ reports
        // here as is (the write socket reports the payload size). A larger
        // write request would become a long write, split across ATT PDUs.
        let size = frame(notify.mtu().saturating_sub(3));
        eprintln!("blessh: {name} {addr}: gatt, frame {size}, writes with response");
        let link = Link::new(size, args.window);
        relay_gatt(link, GattRx(notify), RequestTx(chars.rx.clone())).await
    } else {
        let writer = chars.rx.write_io().await?;
        let size = frame(writer.mtu());
        eprintln!("blessh: {name} {addr}: gatt, frame {size}");
        let link = Link::new(size, args.window);
        relay_gatt(link, GattRx(notify), GattTx(Arc::new(writer))).await
    }
}

/// Receive MTU of the client's last attempts on each channel: small SDUs
/// from the server avoid the flow control stall some sending stacks
/// (CoreBluetooth) run into towards receivers without LE Data Length
/// Extension.
const SMALL_MTU: u16 = 64;

/// Opens the first channel that passes the handshake: each of the
/// server's channels with the client's receive MTU, then each with a small
/// one. None if none does; an error if the server refuses the session.
async fn open_channel(
    local: Address,
    addr: Address,
    peer_type: bluer::AddressType,
    psms: &[u16],
    args: &ProxyArgs,
) -> Result<Option<bluer::l2cap::Stream>> {
    let mut attempts: Vec<(u16, u16)> = psms.iter().map(|p| (*p, args.l2cap_mtu)).collect();
    if args.l2cap_mtu > SMALL_MTU {
        attempts.extend(psms.iter().map(|p| (*p, SMALL_MTU)));
    }
    let attempts = crate::cache::order(&addr.to_string(), attempts);
    let total = attempts.len();
    let _opening = crate::cache::OpenLock::take(&addr.to_string()).await;
    for (i, (psm, mtu)) in attempts.into_iter().enumerate() {
        let next = if i + 1 < total {
            "trying the next channel"
        } else {
            "no channel left"
        };
        let mut stream =
            match bluez::l2cap_connect(local, addr, peer_type, psm, mtu, L2CAP_CONNECT_TIMEOUT)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!("L2CAP channel on PSM {psm} (MTU {mtu}): {e}; {next}");
                    continue;
                }
            };
        match channel::client(
            &mut stream,
            args.probe,
            Duration::from_secs(args.stall_timeout),
        )
        .await
        {
            Ok(ok) => {
                debug!(
                    "channel on PSM {psm} (MTU {mtu}): handshake version {} in {} ms",
                    ok.version,
                    ok.elapsed.as_millis()
                );
                crate::cache::put(&addr.to_string(), psm, mtu);
                return Ok(Some(stream));
            }
            Err(channel::ClientError::Refused(r)) => return Err(r.into()),
            Err(e) => warn!("L2CAP channel on PSM {psm} (MTU {mtu}): {e}; {next}"),
        }
    }
    Ok(None)
}

/// Relays stdin/stdout over the L2CAP channel; returns the exit code.
async fn relay_channel(stream: bluer::l2cap::Stream) -> Result<i32> {
    let stdio = bridge::stdio()?;
    let out = relay::run(stream, stdio, bluez::is_link_end, stop_signal()?).await?;
    debug!(
        "session ended: {:?}, {} bytes from the server, {} bytes to it",
        out.end, out.from_peer, out.to_peer
    );
    Ok(match out.end {
        End::Local | End::Cancelled | End::Peer(None) => 0,
        End::Peer(Some(e)) if e.raw_os_error() == Some(libc::ETIMEDOUT) => {
            warn!("link lost");
            3
        }
        End::Peer(Some(e)) => {
            debug!("channel closed: {}", bluez::link_end_reason(&e));
            0
        }
    })
}

/// Runs the GATT session over an open link; returns the exit code.
async fn relay_gatt<T: PacketTx>(mut link: Link, mut rx: GattRx, mut tx: T) -> Result<i32> {
    let initial = bridge::handshake(&mut link, &mut rx, &mut tx, Duration::from_secs(10)).await?;
    let stdio = bridge::stdio()?;
    let outcome = bridge::run(&mut link, &mut rx, &mut tx, stdio, initial, stop_signal()?).await?;
    if matches!(outcome, Outcome::Cancelled) {
        link.close(CloseReason::Normal, "client exiting");
        while let Some(p) = link.poll_transmit() {
            if tx.send(&p).await.is_err() {
                break;
            }
        }
    }
    let code = match outcome {
        Outcome::StreamEnded | Outcome::Cancelled => 0,
        Outcome::PeerClosed {
            reason: CloseReason::Normal,
            ..
        } => 0,
        Outcome::PeerClosed { reason, message } => {
            warn!("server closed the link: {reason:?} {}", printable(&message));
            2
        }
        Outcome::LinkLost => {
            warn!("link lost");
            3
        }
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::eatt_channels;

    #[test]
    fn reads_gatt_channels() {
        assert_eq!(eatt_channels(""), None);
        assert_eq!(eatt_channels("[GATT]\n#Channels = 3\n"), None);
        assert_eq!(eatt_channels("[GATT]\nChannels = 1\n"), Some(1));
        assert_eq!(
            eatt_channels("[General]\nChannels=2\n[GATT]\nCache = always\n"),
            None
        );
        assert_eq!(eatt_channels("[GATT]\nChannels=3\n[LE]\n"), Some(3));
    }
}
