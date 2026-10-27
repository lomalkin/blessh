//! blessh-loopback: runs the client and server halves of BleStream in one
//! process over a simulated BLE link (small MTU, optional reordering), with
//! stdin/stdout on the client side. Used to test the full SSH path without
//! a radio:
//!
//!   ssh -o ProxyCommand='blessh-loopback --exec "/usr/sbin/sshd -i -f cfg"' user@test

use std::{io, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use blessh_core::{
    bridge::{self, PacketRx, PacketTx},
    link::Link,
    logging,
    session::{self, Preamble},
};
use clap::Parser;
use log::info;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::UnixStream,
    sync::{mpsc, Mutex},
};

#[derive(Parser, Debug)]
struct Args {
    /// Session command; its stdin/stdout become the session stream.
    #[arg(long, conflicts_with = "socket")]
    exec: Option<String>,
    /// Session socket (like the daemon's --session-socket); a session
    /// preamble with a dummy device address is sent first.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Simulated maximum frame size (ATT value).
    #[arg(long, default_value_t = 20)]
    mtu: usize,
    /// Swap every N-th pair of packets in each direction (0 = never).
    #[arg(long, default_value_t = 0)]
    reorder_every: u64,
    #[arg(long, default_value_t = 512)]
    client_window: u32,
    #[arg(long, default_value_t = 512)]
    server_window: u32,
}

/// One direction of the simulated air interface.
struct AirTx {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    held: Arc<Mutex<Option<Vec<u8>>>>,
    count: u64,
    reorder_every: u64,
}

struct AirRx(mpsc::UnboundedReceiver<Vec<u8>>);

impl PacketTx for AirTx {
    async fn send(&mut self, pkt: &[u8]) -> io::Result<()> {
        self.count += 1;
        let gone = || io::Error::from(io::ErrorKind::BrokenPipe);
        let mut held = self.held.lock().await;
        if let Some(prev) = held.take() {
            // Deliver the newer packet first, then the held one.
            self.tx.send(pkt.to_vec()).map_err(|_| gone())?;
            self.tx.send(prev).map_err(|_| gone())?;
        } else if self.reorder_every > 0 && self.count % self.reorder_every == 0 {
            *held = Some(pkt.to_vec());
            // Release a held packet if nothing follows soon.
            let (held, tx) = (self.held.clone(), self.tx.clone());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                if let Some(p) = held.lock().await.take() {
                    let _ = tx.send(p);
                }
            });
        } else {
            self.tx.send(pkt.to_vec()).map_err(|_| gone())?;
        }
        Ok(())
    }
}

impl PacketRx for AirRx {
    async fn recv(&mut self) -> io::Result<Vec<u8>> {
        Ok(self.0.recv().await.unwrap_or_default())
    }
}

fn air(reorder_every: u64) -> (AirTx, AirRx) {
    // Unbounded like bluetoothd's ATT queue; the credit window bounds it.
    let (tx, rx) = mpsc::unbounded_channel();
    (
        AirTx {
            tx,
            held: Arc::new(Mutex::new(None)),
            count: 0,
            reorder_every,
        },
        AirRx(rx),
    )
}

async fn server_half<S: AsyncRead + AsyncWrite>(
    mut link: Link,
    mut rx: AirRx,
    mut tx: AirTx,
    stream: S,
) {
    let initial = match bridge::handshake(&mut link, &mut rx, &mut tx, Duration::from_secs(5)).await
    {
        Ok(i) => i,
        Err(e) => return eprintln!("loopback server: {e}"),
    };
    match bridge::run(
        &mut link,
        &mut rx,
        &mut tx,
        stream,
        initial,
        std::future::pending(),
    )
    .await
    {
        Ok(o) => info!("server half: {o:?}"),
        Err(e) => eprintln!("loopback server: {e}"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logging::init("warn");
    let args = Args::parse();
    let (c2s_tx, c2s_rx) = air(args.reorder_every);
    let (s2c_tx, s2c_rx) = air(args.reorder_every);

    let server_link = Link::new(args.mtu, args.server_window);
    let server = if let Some(cmd) = &args.exec {
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stream = tokio::io::join(child.stdout.take().unwrap(), child.stdin.take().unwrap());
        tokio::spawn(async move {
            server_half(server_link, c2s_rx, s2c_tx, stream).await;
            drop(child);
        })
    } else if let Some(path) = &args.socket {
        let mut stream = UnixStream::connect(path).await?;
        let preamble = Preamble {
            id: session::new_id(),
            device: "00:00:00:00:00:00".into(),
            address_type: "unknown".into(),
        };
        stream.write_all(preamble.encode().as_bytes()).await?;
        tokio::spawn(server_half(server_link, c2s_rx, s2c_tx, stream))
    } else {
        return Err("either --exec or --socket is required".into());
    };

    let mut link = Link::new(args.mtu, args.client_window);
    let (mut rx, mut tx) = (s2c_rx, c2s_tx);
    let initial = bridge::handshake(&mut link, &mut rx, &mut tx, Duration::from_secs(5)).await?;
    let stdio = bridge::stdio()?;
    let outcome = bridge::run(
        &mut link,
        &mut rx,
        &mut tx,
        stdio,
        initial,
        std::future::pending(),
    )
    .await?;
    info!("client half: {outcome:?}");
    drop(tx);
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
    Ok(())
}
