//! The server's bridge (handshake + run) against a hostile BLE peer and a
//! session stream that writes fuzzed output. Time is virtual. Once the peer
//! goes away, the bridge must return promptly, without panicking; it must
//! never send a frame larger than the link's maximum.
#![no_main]

use std::{io, time::Duration};

use arbitrary::Arbitrary;
use ble_term::{
    bridge::{self, PacketRx, PacketTx},
    link::Link,
    protocol::{CloseReason, Frame, MAX_FRAME},
};
use libfuzzer_sys::fuzz_target;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

#[derive(Arbitrary, Debug)]
enum PeerFrame {
    Hello { version: u8, window: u32 },
    Data(Vec<u8>),
    Credit(u32),
    Close(u8),
}

#[derive(Arbitrary, Debug)]
enum Packet {
    Raw(Vec<u8>),
    Frame(PeerFrame, u8),
    /// Let virtual time pass (milliseconds).
    Wait(u16),
}

#[derive(Arbitrary, Debug)]
struct Input {
    max_frame: u16,
    window: u16,
    peer: Vec<Packet>,
    /// What the session (sshd) writes, chunk by chunk.
    session_out: Vec<Vec<u8>>,
    /// The session closes its stream after writing.
    session_ends: bool,
}

struct Rx(mpsc::UnboundedReceiver<Vec<u8>>);

impl PacketRx for Rx {
    async fn recv(&mut self) -> io::Result<Vec<u8>> {
        // An empty packet means the link is gone, as with BlueZ.
        Ok(self.0.recv().await.unwrap_or_default())
    }
}

struct Tx(usize);

impl PacketTx for Tx {
    async fn send(&mut self, pkt: &[u8]) -> io::Result<()> {
        assert!(pkt.len() <= self.0, "frame of {} bytes, limit {}", pkt.len(), self.0);
        assert!(Frame::decode(pkt).is_ok(), "undecodable frame sent");
        Ok(())
    }
}

fuzz_target!(|input: Input| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    rt.block_on(async move {
        let mut link = Link::new(input.max_frame as usize, input.window as u32 + 1);
        let limit = link.max_frame();
        let (peer_tx, peer_rx) = mpsc::unbounded_channel();
        let mut rx = Rx(peer_rx);
        let mut tx = Tx(limit);

        let peer = async move {
            let mut seq: u16 = 0;
            let mut waited = Duration::ZERO;
            for p in input.peer {
                let pkt = match p {
                    Packet::Raw(b) => b,
                    Packet::Frame(f, skew) => {
                        let f = match f {
                            PeerFrame::Hello { version, window } => Frame::Hello { version, flags: 0, window },
                            PeerFrame::Data(mut d) => {
                                d.truncate(MAX_FRAME - 3);
                                Frame::Data(d)
                            }
                            PeerFrame::Credit(n) => Frame::Credit(n),
                            PeerFrame::Close(r) => Frame::Close { reason: CloseReason::from_u8(r), message: String::new() },
                        };
                        let s = seq.wrapping_add(skew as u16);
                        if skew == 0 {
                            seq = seq.wrapping_add(1);
                        }
                        f.encode(s)
                    }
                    Packet::Wait(ms) => {
                        // At most 10 virtual minutes in total, so that the
                        // hang detection below stays meaningful.
                        let d = Duration::from_millis(ms as u64);
                        if waited + d <= Duration::from_secs(600) {
                            waited += d;
                            tokio::time::sleep(d).await;
                        }
                        continue;
                    }
                };
                if peer_tx.send(pkt).is_err() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            // Dropping peer_tx: the link goes away.
        };

        let (session_side, bridge_side) = tokio::io::duplex(4096);
        let session = async move {
            let (mut r, mut w) = tokio::io::split(session_side);
            let reader = async move {
                let mut buf = [0u8; 1024];
                while matches!(r.read(&mut buf).await, Ok(n) if n > 0) {}
            };
            let writer = async move {
                for chunk in input.session_out {
                    if w.write_all(&chunk).await.is_err() {
                        return;
                    }
                }
                if input.session_ends {
                    let _ = w.shutdown().await;
                } else {
                    std::future::pending::<()>().await;
                }
            };
            tokio::join!(reader, writer);
        };

        let server = async move {
            let initial = match bridge::handshake(&mut link, &mut rx, &mut tx, Duration::from_secs(20)).await {
                Ok(ev) => ev,
                Err(_) => return,
            };
            let _ = bridge::run(&mut link, &mut rx, &mut tx, bridge_side, initial, std::future::pending()).await;
        };

        tokio::spawn(session);
        tokio::spawn(peer);
        // Virtual time: the handshake timeout and the peer's waits add up to
        // well under an hour, so anything longer is a hang.
        tokio::time::timeout(Duration::from_secs(3600), server)
            .await
            .expect("bridge did not return after the peer went away");
    });
});
