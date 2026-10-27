//! ble-term-connect: desktop client that connects to a ble-term server and
//! relays the session over stdin/stdout. Intended as an OpenSSH ProxyCommand:
//!
//!   ssh -o ProxyCommand='ble-term-connect AA:BB:CC:DD:EE:FF' user@pi

use std::{collections::HashSet, sync::Arc, time::Duration};

use ble_term::{
    bridge::{self, Outcome, PacketRx, PacketTx},
    link::Link,
    logging,
    protocol::{self, printable, CloseReason, INFO_UUID, RX_UUID, SERVICE_UUID, TX_UUID},
};
use bluer::{
    gatt::{
        remote::{Characteristic, CharacteristicWriteRequest},
        WriteOp,
    },
    Adapter, AdapterEvent, Address, Device, DiscoveryFilter, DiscoveryTransport,
};
use clap::Parser;
use futures::StreamExt;
use log::{debug, info, warn};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Connect to a ble-term server and relay stdin/stdout (OpenSSH ProxyCommand)"
)]
struct Args {
    /// Server address (AA:BB:CC:DD:EE:FF) or advertised name; omit to use the first server found.
    target: Option<String>,
    /// Bluetooth adapter to use.
    #[arg(long)]
    adapter: Option<String>,
    /// Seconds to scan for the server.
    #[arg(long, default_value_t = 15)]
    scan_timeout: u64,
    /// Only list servers found during the scan.
    #[arg(long)]
    scan: bool,
    /// Receive window granted to the server, in bytes.
    #[arg(long, default_value_t = 32768)]
    window: u32,
    /// Upper limit for frames written to the server (default: from the ATT MTU).
    #[arg(long)]
    max_frame: Option<usize>,
    /// Leave the BLE link up on exit, like a phone that keeps it for a few
    /// seconds (for tests).
    #[arg(long, hide = true)]
    keep_link: bool,
    /// Write frames with response instead of without (for tests: clients
    /// whose platform loses writes without response use this).
    #[arg(long, hide = true)]
    write_with_response: bool,
}

struct Rx(bluer::gatt::CharacteristicReader);

impl PacketRx for Rx {
    async fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        self.0.recv().await
    }
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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    logging::init("warn");
    let args = Args::parse();
    match run(args).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("ble-term-connect: {e}");
            std::process::exit(1);
        }
    }
}

async fn find(adapter: &Adapter, args: &Args) -> bluer::Result<Option<Device>> {
    if let Some(addr) = args
        .target
        .as_deref()
        .and_then(|t| t.parse::<Address>().ok())
    {
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
    let deadline = tokio::time::sleep(Duration::from_secs(args.scan_timeout));
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
            let target_addr = args
                .target
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&addr.to_string()));
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
            if args.scan {
                if listed.insert(addr) {
                    println!(
                        "{addr}  {name}  rssi={}",
                        dev.rssi().await.ok().flatten().unwrap_or_default()
                    );
                }
                continue;
            }
            match args.target.as_deref() {
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

/// Why this machine's BlueZ may open several ATT bearers (EATT), which
/// BleStream v1 does not support. BlueZ before 5.67 does so by default.
fn eatt_hint() -> Option<String> {
    // EATT needs the kernel's ECRED L2CAP mode; the ble-term package turns
    // it off on BlueZ before 5.67 (README, "Bookworm").
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

/// Waits until no other ble-term-connect of this user uses the device, and
/// keeps it that way while the returned file is open. ssh does not wait for
/// its ProxyCommand to exit, so a second connection made right away can start
/// while the previous client is still disconnecting; sharing the link
/// breaks the new session.
async fn lock_device(addr: Address) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let name = format!(
        "ble-term-connect-{}.lock",
        addr.to_string().replace(':', "")
    );
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(&name))
    {
        Ok(f) => f,
        Err(e) => {
            debug!("cannot open lock file {name}: {e}");
            return None;
        }
    };
    for attempt in 0..100 {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Some(file);
        }
        if attempt == 0 {
            info!("waiting for another ble-term-connect to release {addr}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    warn!("another ble-term-connect still uses {addr}, connecting anyway");
    None
}

async fn run(args: Args) -> Result<i32, Box<dyn std::error::Error>> {
    if let Some(why) = eatt_hint() {
        warn!(
            "{why}; BleStream needs a single ATT bearer: set Channels = 1 in the [GATT] section \
             of /etc/bluetooth/main.conf and restart bluetooth"
        );
    }
    let keep_link = args.keep_link;
    let session = bluer::Session::new().await?;
    let adapter = match &args.adapter {
        Some(n) => session.adapter(n)?,
        None => session.default_adapter().await?,
    };
    if !adapter.is_powered().await? {
        return Err(format!("adapter {} is powered off", adapter.name()).into());
    }
    let Some(dev) = find(&adapter, &args).await? else {
        if args.scan {
            return Ok(0);
        }
        return Err("no ble-term server found".into());
    };
    let _lock = lock_device(dev.address()).await;
    info!("connecting to {}", dev.address());
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
    let Some(chars) = characteristics(&dev).await? else {
        return Err("device does not provide the ble-term service".into());
    };
    if let Some(c) = &chars.info {
        match c.read().await {
            Ok(v) => {
                let v = String::from_utf8_lossy(&v);
                info!("server info: {}", printable(&v));
                // Subscribing would make things worse for the client that
                // holds the server (see the daemon); leave without it.
                if protocol::info_busy(&v) {
                    if !keep_link {
                        let _ = dev.disconnect().await;
                    }
                    return Err("server is busy: another client is connected".into());
                }
            }
            Err(e) => debug!("cannot read INFO: {e}"),
        }
    }
    let notify = chars.tx.notify_io().await?;
    let frame = |mtu: usize| args.max_frame.map_or(mtu, |m| m.min(mtu));
    let code = if args.write_with_response {
        // The write socket is not acquired: BlueZ refuses WriteValue while
        // it is held. Both directions share the ATT MTU, which BlueZ reports
        // here as is (the write socket reports the payload size). A larger
        // write request would become a long write, split across ATT PDUs.
        info!(
            "link up: notify MTU {}, writing with response",
            notify.mtu()
        );
        let link = Link::new(frame(notify.mtu().saturating_sub(3)), args.window);
        relay(link, Rx(notify), RequestTx(chars.rx)).await?
    } else {
        let writer = chars.rx.write_io().await?;
        info!(
            "link up: write MTU {}, notify MTU {}",
            writer.mtu(),
            notify.mtu()
        );
        let link = Link::new(frame(writer.mtu()), args.window);
        relay(link, Rx(notify), Arc::new(writer)).await?
    };
    if !keep_link {
        let _ = dev.disconnect().await;
    }
    Ok(code)
}

/// Runs the session over an open link; returns the exit code.
async fn relay<T: PacketTx>(
    mut link: Link,
    mut rx: Rx,
    mut tx: T,
) -> Result<i32, Box<dyn std::error::Error>> {
    let initial = bridge::handshake(&mut link, &mut rx, &mut tx, Duration::from_secs(10)).await?;
    let stdio = bridge::stdio()?;
    // ssh terminates its ProxyCommand with SIGHUP/SIGTERM when it is done.
    let mut hup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let stop = async move {
        tokio::select! {
            _ = hup.recv() => {}
            _ = term.recv() => {}
        }
    };
    let outcome = bridge::run(&mut link, &mut rx, &mut tx, stdio, initial, stop).await?;
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
