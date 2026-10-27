//! Linux client: BlueZ through bluer. Finds the server, connects, reads
//! INFO and runs the session over an L2CAP channel, or over the GATT
//! stream when the server offers no channel or `--gatt` asks for it.

use std::{collections::HashSet, error::Error, future::Future, sync::Arc, time::Duration};

use blessh_ble::bluez::{self, GattRx, GattTx};
use blessh_core::{
    bridge::{self, Outcome, PacketTx},
    link::Link,
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

/// How long opening the L2CAP channel may take, link setup included.
const L2CAP_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

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
    match rt.block_on(f) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("blessh: {e}");
            1
        }
    }
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
    find(&adapter, None, args.timeout, true).await?;
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

/// Finds the server: by address or advertised name, or the first one seen
/// when `target` is None. With `list`, prints every server instead.
async fn find(
    adapter: &Adapter,
    target: Option<&str>,
    timeout: u64,
    list: bool,
) -> bluer::Result<Option<Device>> {
    if let Some(addr) = target.and_then(|t| t.parse::<Address>().ok()) {
        if let Ok(dev) = adapter.device(addr) {
            if dev.is_connected().await.unwrap_or(false)
                || dev.uuids().await.ok().flatten().is_some()
            {
                return Ok(Some(dev));
            }
        }
    }
    adapter
        .set_discovery_filter(DiscoveryFilter {
            uuids: [SERVICE_UUID].into_iter().collect(),
            transport: DiscoveryTransport::Le,
            ..Default::default()
        })
        .await?;
    let events = adapter.discover_devices().await?;
    tokio::pin!(events);
    let deadline = tokio::time::sleep(Duration::from_secs(timeout));
    tokio::pin!(deadline);
    // Advertised UUIDs and names can arrive after DeviceAdded, so devices
    // are re-checked until they match or the scan ends.
    // Devices BlueZ already knows produce no DeviceAdded event.
    let mut candidates: Vec<Address> = adapter.device_addresses().await?;
    let mut listed: HashSet<Address> = HashSet::new();
    let mut recheck = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = &mut deadline => return Ok(None),
            ev = events.next() => {
                if let Some(AdapterEvent::DeviceAdded(addr)) = ev {
                    if !candidates.contains(&addr) {
                        candidates.push(addr);
                    }
                }
            }
            _ = recheck.tick() => {}
        }
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
            if !advertises && !target_addr {
                continue;
            }
            let name = dev.name().await.ok().flatten().unwrap_or_default();
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
                None => return Ok(Some(dev)),
                Some(t) if target_addr || t == name => return Ok(Some(dev)),
                _ => {}
            }
        }
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
    // off on BlueZ before 5.67 (README, "Bookworm").
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

/// Keeps the blessh instances of one user that use the same device from
/// stepping on each other's link: each holds a shared lock while connected
/// (several sessions share one link), and the last one to leave takes the
/// exclusive lock and disconnects. ssh does not wait for its ProxyCommand
/// to exit, so a new session can start while the previous one leaves.
struct DeviceLock(Option<std::fs::File>);

impl DeviceLock {
    async fn shared(addr: Address) -> Self {
        use std::os::fd::AsRawFd;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let name = format!("blessh-{}.lock", addr.to_string().replace(':', ""));
        let file = match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(&name))
        {
            Ok(f) => f,
            Err(e) => {
                debug!("cannot open lock file {name}: {e}");
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
    fn last_user(&self) -> bool {
        use std::os::fd::AsRawFd;
        match &self.0 {
            Some(f) => (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) == 0,
            None => true,
        }
    }
}

async fn connect(dev: &Device) -> bluer::Result<()> {
    if dev.is_connected().await? {
        return Ok(());
    }
    let mut attempt = 0;
    loop {
        match dev.connect().await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 2 => {
                debug!("connect failed: {e}, retrying");
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn proxy_async(args: ProxyArgs) -> Result<i32> {
    let adapter = adapter(args.adapter.as_deref()).await?;
    let Some(dev) = find(&adapter, args.target.as_deref(), args.scan_timeout, false).await? else {
        return Err("no blessh server found".into());
    };
    let addr = dev.address();
    let name = dev.name().await.ok().flatten().unwrap_or_default();
    let lock = DeviceLock::shared(addr).await;
    info!("connecting to {addr}");
    let result = session(&adapter, &dev, &args, &name).await;
    if !args.keep_link && lock.last_user() {
        let _ = dev.disconnect().await;
    }
    result
}

/// Connects and runs the session over the best transport the server offers.
async fn session(adapter: &Adapter, dev: &Device, args: &ProxyArgs, name: &str) -> Result<i32> {
    let addr = dev.address();
    connect(dev).await?;
    let Some(chars) = characteristics(dev).await? else {
        return Err("device does not provide the blessh service".into());
    };
    let info = match &chars.info {
        Some(c) => match c.read().await {
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
        None => None,
    };
    let psm = info.as_deref().and_then(protocol::info_psm);
    if let Some(psm) = psm.filter(|_| !args.gatt) {
        let local = adapter.address().await?;
        let peer_type = dev.address_type().await?;
        match bluez::l2cap_connect(
            local,
            addr,
            peer_type,
            psm,
            args.l2cap_mtu,
            L2CAP_CONNECT_TIMEOUT,
        )
        .await
        {
            Ok(stream) => {
                let mtu = stream.as_ref().send_mtu().unwrap_or(0);
                eprintln!("blessh: {name} {addr}: l2cap, mtu {mtu}");
                return relay_channel(stream).await;
            }
            Err(e) => warn!("L2CAP channel on PSM {psm} failed: {e}; using the GATT stream"),
        }
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

/// Relays stdin/stdout over the L2CAP channel; returns the exit code.
async fn relay_channel(stream: bluer::l2cap::Stream) -> Result<i32> {
    let stdio = bridge::stdio()?;
    let out = relay::run(stream, stdio, bluez::is_link_end, stop_signal()?).await?;
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
