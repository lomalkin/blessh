//! macOS client: CoreBluetooth through blessh-ble. Servers are selected by
//! advertised name or by CoreBluetooth's device identifier (macOS shows no
//! Bluetooth addresses); sessions run over the L2CAP channel.

use std::{collections::HashSet, error::Error, io, time::Duration};

use blessh_ble::corebluetooth::{
    self,
    central::{Central, Found},
    stream::L2capStream,
};
use blessh_core::{
    bridge, channel, name,
    protocol::{self, printable, INFO_UUID},
    relay::{self, End},
};
use log::{info, warn};

use crate::cli::{stop_signal, ProxyArgs, ScanArgs};

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

fn scan_sync(args: &ScanArgs) -> Result<()> {
    let central = Central::new(BLUETOOTH_TIMEOUT)?;
    // A server is listed once its advertised name is known; those whose
    // scan response never came are listed at the end with the name
    // CoreBluetooth has cached.
    let mut listed = HashSet::new();
    let mut pending: Vec<(String, String, i64)> = Vec::new();
    central.scan(Duration::from_secs(args.timeout), |f| {
        if listed.contains(&f.id) {
            return false;
        }
        match &f.local_name {
            Some(n) => {
                println!("{}  {n}  rssi={}", f.id, f.rssi);
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
    for (id, n, rssi) in pending {
        println!("{id}  {n}  rssi={rssi}");
    }
    Ok(())
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

/// Finds the server, connects and opens the channel. CoreBluetooth calls
/// block, so this runs off the async runtime.
fn open(target: Option<String>, scan_timeout: Duration) -> io::Result<(Central, Found, Vec<u16>)> {
    let central = Central::new(BLUETOOTH_TIMEOUT)?;
    // Servers in range by identifier, with the best name known so far.
    let mut seen: Vec<(String, String)> = Vec::new();
    let found = central
        .scan(scan_timeout, |f| {
            let n = f.name().unwrap_or_default().to_owned();
            match seen.iter_mut().find(|(id, _)| *id == f.id) {
                Some(e) if f.local_name.is_some() => e.1 = n,
                Some(_) => {}
                None => seen.push((f.id.clone(), n)),
            }
            // Only the advertised name counts: the cached one may be stale.
            match &target {
                None => true,
                Some(t) => {
                    f.id.eq_ignore_ascii_case(t)
                        || f.local_name.as_deref().is_some_and(|n| name::same(n, t))
                }
            }
        })?
        .ok_or_else(|| {
            let what = match &target {
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
        })?;
    central.connect(&found.peripheral, STEP_TIMEOUT)?;
    let chars = central.discover(&found.peripheral, STEP_TIMEOUT)?;
    let info_char = chars
        .iter()
        .find(|(uuid, _)| *uuid == INFO_UUID.to_string())
        .map(|(_, c)| c.clone())
        .ok_or_else(|| io::Error::other("device does not provide the blessh service"))?;
    let info =
        String::from_utf8_lossy(&central.read(&found.peripheral, &info_char, STEP_TIMEOUT)?)
            .into_owned();
    info!("server info: {}", printable(&info));
    let psms = protocol::info_psms(&info);
    if psms.is_empty() {
        return Err(io::Error::other(
            "the server offers no L2CAP channel; the GATT stream is not available on macOS yet",
        ));
    }
    Ok((central, found, psms))
}

/// Opens the first of the server's channels that passes the handshake.
/// CoreBluetooth calls block, so they run off the async runtime; the
/// central travels with them.
async fn open_channel(
    mut central: Central,
    found: &Found,
    psms: &[u16],
    args: &ProxyArgs,
) -> Result<(Central, L2capStream, u16)> {
    let attempts = crate::cache::order(&found.id, psms.iter().map(|p| (*p, 0)).collect());
    let _opening = crate::cache::OpenLock::take(&found.id).await;
    for (i, &(psm, _)) in attempts.iter().enumerate() {
        let next = if i + 1 < attempts.len() {
            "trying the next channel"
        } else {
            "no channel left"
        };
        let peripheral = found.peripheral.clone();
        let (c, opened) = tokio::task::spawn_blocking(move || {
            let r = central.open_channel(&peripheral, psm, STEP_TIMEOUT);
            (central, r)
        })
        .await?;
        central = c;
        let mut stream = match opened {
            Ok(s) => s,
            Err(e) => {
                warn!("L2CAP channel on PSM {psm}: {e}; {next}");
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
                log::debug!(
                    "channel on PSM {psm}: handshake version {} in {} ms",
                    ok.version,
                    ok.elapsed.as_millis()
                );
                crate::cache::put(&found.id, psm, 0);
                return Ok((central, stream, psm));
            }
            Err(channel::ClientError::Refused(r)) => return Err(r.into()),
            Err(e) => warn!("L2CAP channel on PSM {psm}: {e}; {next}"),
        }
    }
    Err(
        "no L2CAP channel of the server works, and the GATT stream is not available on macOS yet"
            .into(),
    )
}

async fn proxy_async(args: ProxyArgs) -> Result<i32> {
    if args.gatt {
        return Err("the GATT stream is not available on macOS yet".into());
    }
    let (target, scan_timeout) = (args.target.clone(), Duration::from_secs(args.scan_timeout));
    let (central, found, psms) =
        tokio::task::spawn_blocking(move || open(target, scan_timeout)).await??;
    let (central, stream, psm) = open_channel(central, &found, &psms, &args).await?;
    eprintln!(
        "blessh: {} {}: l2cap, psm {psm}",
        found.name().unwrap_or_default(),
        found.id
    );
    let stdio = bridge::stdio()?;
    let outcome = relay::run(stream, stdio, corebluetooth::is_link_end, stop_signal()?).await;
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
