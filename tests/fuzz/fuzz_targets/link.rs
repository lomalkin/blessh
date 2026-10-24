//! One link driven by a hostile peer: arbitrary packets (raw, or well-formed
//! frames with fuzzed fields so the state machine gets past the handshake)
//! interleaved with the application's calls. The link may reject input with
//! an error but must never panic, and its bookkeeping must stay consistent.
#![no_main]

use arbitrary::Arbitrary;
use ble_term::{
    link::{Event, Link},
    protocol::{CloseReason, Frame, MAX_FRAME},
};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum PeerFrame {
    Hello { version: u8, flags: u8, window: u32 },
    Data(Vec<u8>),
    Credit(u32),
    Close(u8, String),
}

#[derive(Arbitrary, Debug)]
enum Op {
    /// Raw bytes from the peer.
    Raw(Vec<u8>),
    /// A frame from the peer; `seq_skew` 0 means the expected sequence number.
    Frame { frame: PeerFrame, seq_skew: u8 },
    /// The application writes stream data.
    Send(Vec<u8>),
    /// The application consumes up to `n` delivered bytes.
    Consume(u16),
    /// The transport sends whatever the link has queued.
    Transmit,
    Close(u8),
}

#[derive(Arbitrary, Debug)]
struct Input {
    max_frame: u16,
    window: u16,
    ops: Vec<Op>,
}

fuzz_target!(|input: Input| {
    let max_frame = input.max_frame as usize;
    let mut link = Link::new(max_frame, input.window as u32 + 1);
    let mut peer_seq: u16 = 0;
    let mut unconsumed: usize = 0;
    for op in input.ops {
        let result = match op {
            Op::Raw(pkt) => link.on_packet(&pkt),
            Op::Frame { frame, seq_skew } => {
                let frame = match frame {
                    PeerFrame::Hello { version, flags, window } => Frame::Hello { version, flags, window },
                    PeerFrame::Data(mut d) => {
                        d.truncate(MAX_FRAME - 3);
                        Frame::Data(d)
                    }
                    PeerFrame::Credit(n) => Frame::Credit(n),
                    PeerFrame::Close(r, m) => Frame::Close { reason: CloseReason::from_u8(r), message: m },
                };
                let seq = peer_seq.wrapping_add(seq_skew as u16);
                if seq_skew == 0 {
                    peer_seq = peer_seq.wrapping_add(1);
                }
                link.on_packet(&frame.encode(seq))
            }
            Op::Send(d) => {
                link.send(&d);
                Ok(Vec::new())
            }
            Op::Consume(n) => {
                let n = (n as usize).min(unconsumed);
                unconsumed -= n;
                link.consumed(n);
                Ok(Vec::new())
            }
            Op::Transmit => {
                while let Some(pkt) = link.poll_transmit() {
                    assert!(pkt.len() <= link.max_frame(), "frame larger than max_frame");
                    assert!(Frame::decode(&pkt).is_ok(), "link sent an undecodable frame");
                }
                Ok(Vec::new())
            }
            Op::Close(r) => {
                link.close(CloseReason::from_u8(r), "fuzz");
                Ok(Vec::new())
            }
        };
        match result {
            Ok(events) => {
                link.check_invariants();
                for ev in events {
                    if let Event::Data(d) = ev {
                        assert!(!d.is_empty());
                        unconsumed += d.len();
                    }
                }
            }
            // A real endpoint closes the link here.
            Err(_) => return,
        }
    }
});
