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
    bridge,
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
    let mut seen = HashSet::new();
    central.scan(Duration::from_secs(args.timeout), |f| {
        if seen.insert(f.id.clone()) {
            println!(
                "{}  {}  rssi={}",
                f.id,
                f.name.clone().unwrap_or_default(),
                f.rssi
            );
        }
        false
    })?;
    Ok(())
}

pub fn proxy(args: ProxyArgs) -> i32 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match rt.block_on(proxy_async(args)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("blessh: {e}");
            1
        }
    }
}

/// Finds the server, connects and opens the channel. CoreBluetooth calls
/// block, so this runs off the async runtime.
fn open(
    target: Option<String>,
    scan_timeout: Duration,
) -> io::Result<(Central, Found, L2capStream, u16)> {
    let central = Central::new(BLUETOOTH_TIMEOUT)?;
    let found = central
        .scan(scan_timeout, |f| match &target {
            None => true,
            Some(t) => f.id.eq_ignore_ascii_case(t) || f.name.as_deref() == Some(t.as_str()),
        })?
        .ok_or_else(|| io::Error::other("no blessh server found"))?;
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
    let psm = protocol::info_psm(&info).ok_or_else(|| {
        io::Error::other(
            "the server offers no L2CAP channel; the GATT stream is not available on macOS yet",
        )
    })?;
    let stream = central.open_channel(&found.peripheral, psm, STEP_TIMEOUT)?;
    Ok((central, found, stream, psm))
}

async fn proxy_async(args: ProxyArgs) -> Result<i32> {
    if args.gatt {
        return Err("the GATT stream is not available on macOS yet".into());
    }
    let (target, scan_timeout) = (args.target.clone(), Duration::from_secs(args.scan_timeout));
    let (central, found, stream, psm) =
        tokio::task::spawn_blocking(move || open(target, scan_timeout)).await??;
    eprintln!(
        "blessh: {} {}: l2cap, psm {psm}",
        found.name.clone().unwrap_or_default(),
        found.id
    );
    let stdio = bridge::stdio()?;
    let outcome = relay::run(stream, stdio, corebluetooth::is_link_end, stop_signal()?).await;
    if !args.keep_link {
        central.disconnect(&found.peripheral);
    }
    Ok(match outcome?.end {
        End::Local | End::Cancelled | End::Peer(None) => 0,
        End::Peer(Some(e)) => {
            warn!("channel ended: {e}");
            3
        }
    })
}
