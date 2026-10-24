//! ble-term-connect: desktop client that connects to a ble-term server and
//! relays the session over stdin/stdout. Intended as an OpenSSH ProxyCommand:
//!
//!   ssh -o ProxyCommand='ble-term-connect AA:BB:CC:DD:EE:FF' user@pi

use std::{collections::HashSet, sync::Arc, time::Duration};

use ble_term::{
    bridge::{self, Outcome, PacketRx, PacketTx},
    link::Link,
    logging,
    protocol::{CloseReason, RX_UUID, SERVICE_UUID, TX_UUID},
};
use bluer::{
    gatt::remote::Characteristic, Adapter, AdapterEvent, Address, Device, DiscoveryFilter,
    DiscoveryTransport,
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
}

struct Rx(bluer::gatt::CharacteristicReader);

impl PacketRx for Rx {
    async fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        self.0.recv().await
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
    let mut candidates: Vec<Address> = Vec::new();
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

async fn characteristics(dev: &Device) -> bluer::Result<Option<(Characteristic, Characteristic)>> {
    for service in dev.services().await? {
        if service.uuid().await? != SERVICE_UUID {
            continue;
        }
        let (mut rx, mut tx) = (None, None);
        for c in service.characteristics().await? {
            match c.uuid().await? {
                u if u == RX_UUID => rx = Some(c),
                u if u == TX_UUID => tx = Some(c),
                _ => {}
            }
        }
        if let (Some(rx), Some(tx)) = (rx, tx) {
            return Ok(Some((rx, tx)));
        }
    }
    Ok(None)
}

async fn run(args: Args) -> Result<i32, Box<dyn std::error::Error>> {
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
    let Some((rx_char, tx_char)) = characteristics(&dev).await? else {
        return Err("device does not provide the ble-term service".into());
    };
    let notify = tx_char.notify_io().await?;
    let writer = rx_char.write_io().await?;
    info!(
        "link up: write MTU {}, notify MTU {}",
        writer.mtu(),
        notify.mtu()
    );

    let mut link = Link::new(
        args.max_frame.map_or(writer.mtu(), |m| m.min(writer.mtu())),
        args.window,
    );
    let mut rx = Rx(notify);
    let mut tx = Arc::new(writer);
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
            warn!("server closed the link: {reason:?} {message}");
            2
        }
        Outcome::LinkLost => {
            warn!("link lost");
            3
        }
    };
    let _ = dev.disconnect().await;
    Ok(code)
}
