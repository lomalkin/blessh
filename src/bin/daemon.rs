//! ble-term-daemon: exposes the BleStream GATT service through BlueZ and
//! forwards each BLE client to a session socket (normally a systemd socket
//! that spawns `sshd -i` per connection).

use std::{path::PathBuf, sync::Arc, time::Duration};

use ble_term::{
    bridge::{self, BridgeError, Outcome, PacketTx},
    link::Link,
    logging,
    protocol::{CloseReason, Frame, INFO_UUID, RX_UUID, SERVICE_UUID, TX_UUID, VERSION},
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
    Adapter, Address, DeviceEvent, DeviceProperty,
};
use clap::Parser;
use futures::{FutureExt, StreamExt};
use log::{debug, error, info, warn};
use tokio::{net::UnixStream, task::JoinHandle, time::Instant};

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
    #[arg(long, default_value_t = 20)]
    handshake_timeout: u64,
    /// Receive window granted to the client, in bytes.
    #[arg(long, default_value_t = 16384)]
    window: u32,
    /// SSH host public key whose fingerprint is logged at start-up.
    #[arg(long, default_value = "/etc/ble-term/ssh_host_ed25519_key.pub")]
    host_key: PathBuf,
}

struct Pending {
    reader: Option<CharacteristicReader>,
    writer: Option<CharacteristicWriter>,
    since: Instant,
}

impl Pending {
    fn new() -> Self {
        Self {
            reader: None,
            writer: None,
            since: Instant::now(),
        }
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

async fn wait_powered(adapter: &Adapter) -> bluer::Result<()> {
    let mut warned = false;
    while !adapter.is_powered().await? {
        if !warned {
            warn!("adapter {} is powered off, waiting", adapter.name());
            warned = true;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    Ok(())
}

async fn advertise(adapter: &Adapter, name: &str) -> bluer::Result<AdvertisementHandle> {
    let adv = Advertisement {
        service_uuids: [SERVICE_UUID].into_iter().collect(),
        discoverable: Some(true),
        local_name: Some(name.to_owned()),
        ..Default::default()
    };
    adapter.advertise(adv).await
}

async fn serve(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let session = bluer::Session::new().await?;
    let adapter = match &args.adapter {
        Some(name) => session.adapter(name)?,
        None => session.default_adapter().await?,
    };
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

    let info = format!("proto={VERSION};name={name}").into_bytes();
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
                        fun: Box::new(move |_| {
                            let v = info.clone();
                            async move { Ok(v) }.boxed()
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
    let mut active: Option<(Address, JoinHandle<()>)> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        tokio::select! {
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            ev = rx_control.next() => match ev {
                Some(CharacteristicControlEvent::Write(req)) => {
                    let addr = req.device_address();
                    if active.as_ref().is_some_and(|(a, _)| *a != addr) || pending.address().is_some_and(|a| a != addr) {
                        info!("{addr}: rejecting, another client is connected");
                        req.reject(ReqError::NotPermitted);
                        disconnect(&adapter, addr).await;
                    } else if active.is_some() {
                        warn!("{addr}: unexpected write re-acquire during a session");
                        req.reject(ReqError::NotPermitted);
                    } else {
                        debug!("{addr}: write acquired, MTU {}", req.mtu());
                        if pending.address().is_none() {
                            pending.since = Instant::now();
                        }
                        pending.reader = Some(req.accept()?);
                    }
                }
                Some(_) => {}
                None => return Err("GATT application was unregistered".into()),
            },
            ev = tx_control.next() => match ev {
                Some(CharacteristicControlEvent::Notify(writer)) => {
                    let addr = writer.device_address();
                    if active.is_some() || pending.address().is_some_and(|a| a != addr) {
                        // Only BlueZ >= 5.69 hands out per-device notify sockets,
                        // so this cannot reach the active client.
                        info!("{addr}: rejecting, another client is connected");
                        let busy = Frame::Close { reason: CloseReason::Busy, message: "another client is connected".into() };
                        let _ = writer.try_send(&busy.encode(0));
                        drop(writer);
                        disconnect(&adapter, addr).await;
                    } else {
                        debug!("{addr}: notifications enabled, MTU {}", writer.mtu());
                        if pending.address().is_none() {
                            pending.since = Instant::now();
                        }
                        pending.writer = Some(writer);
                    }
                }
                Some(_) => {}
                None => return Err("GATT application was unregistered".into()),
            },
            _ = async { (&mut active.as_mut().unwrap().1).await }, if active.is_some() => {
                let (addr, _) = active.take().unwrap();
                debug!("{addr}: session task finished");
                // Some controllers do not resume advertising after a connection.
                adv = None;
                match advertise(&adapter, &name).await {
                    Ok(h) => adv = Some(h),
                    Err(e) => error!("re-registering advertisement failed: {e}"),
                }
            }
            _ = tick.tick() => {
                if adv.is_none() {
                    match advertise(&adapter, &name).await {
                        Ok(h) => adv = Some(h),
                        Err(e) => debug!("advertising retry failed: {e}"),
                    }
                }
                if let Some(addr) = pending.address() {
                    if pending.since.elapsed() > handshake_timeout {
                        info!("{addr}: client did not open both characteristics in time");
                        pending = Pending::new();
                        disconnect(&adapter, addr).await;
                    }
                }
            }
        }

        if active.is_none() && pending.reader.is_some() && pending.writer.is_some() {
            let p = std::mem::replace(&mut pending, Pending::new());
            let (reader, writer) = (p.reader.unwrap(), p.writer.unwrap());
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

async fn disconnect(adapter: &Adapter, addr: Address) {
    if let Ok(dev) = adapter.device(addr) {
        if let Err(e) = dev.disconnect().await {
            debug!("{addr}: disconnect failed: {e}");
        }
    }
}

/// Resolves when the device is no longer connected or the notify socket closes.
async fn link_gone(adapter: Adapter, addr: Address, writer: Arc<CharacteristicWriter>) {
    let watch = async {
        let dev = adapter.device(addr).ok()?;
        let mut events = dev.events().await.ok()?;
        if !dev.is_connected().await.unwrap_or(false) {
            return Some(());
        }
        while let Some(DeviceEvent::PropertyChanged(p)) = events.next().await {
            if let DeviceProperty::Connected(false) = p {
                return Some(());
            }
        }
        Some(())
    };
    tokio::select! {
        _ = watch => {}
        _ = writer.closed() => {}
    }
}

async fn run_session(
    adapter: Adapter,
    addr: Address,
    mut reader: CharacteristicReader,
    writer: CharacteristicWriter,
    args: Args,
) {
    let writer = Arc::new(writer);
    let mut tx = writer.clone();
    let mut link = Link::new(writer.mtu(), args.window);
    info!(
        "{addr}: client connected (notify MTU {}, write MTU {})",
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
        let stream = match UnixStream::connect(&args.session_socket).await {
            Ok(s) => s,
            Err(e) => {
                error!(
                    "{addr}: cannot start session via {}: {e}",
                    args.session_socket.display()
                );
                link.close(CloseReason::SessionUnavailable, "cannot start session");
                while let Some(p) = link.poll_transmit() {
                    tx.send(&p).await?;
                }
                return Ok(None);
            }
        };
        info!("{addr}: session started");
        let gone = link_gone(adapter.clone(), addr, writer.clone());
        bridge::run(&mut link, &mut reader, &mut tx, stream, initial, gone)
            .await
            .map(Some)
    }
    .await;

    let lost = match result {
        Ok(Some(Outcome::StreamEnded)) => {
            info!("{addr}: session ended");
            false
        }
        Ok(Some(Outcome::PeerClosed { reason, message })) => {
            info!("{addr}: client closed the session ({reason:?} {message})");
            false
        }
        Ok(Some(Outcome::LinkLost | Outcome::Cancelled)) => {
            info!("{addr}: client disconnected");
            true
        }
        Ok(None) => false,
        Err(BridgeError::Protocol(e)) => {
            warn!("{addr}: {e}");
            link.close(CloseReason::ProtocolError, &e.to_string());
            while let Some(p) = link.poll_transmit() {
                if tx.send(&p).await.is_err() {
                    break;
                }
            }
            false
        }
        Err(BridgeError::HandshakeTimeout) => {
            info!("{addr}: handshake timed out");
            false
        }
        Err(BridgeError::Io(e)) => {
            info!("{addr}: link error: {e}");
            true
        }
    };
    drop(reader);
    drop(tx);
    drop(writer);
    if !lost {
        // Give the client a moment to read the CLOSE frame.
        tokio::time::sleep(Duration::from_millis(500)).await;
        disconnect(&adapter, addr).await;
    }
}
