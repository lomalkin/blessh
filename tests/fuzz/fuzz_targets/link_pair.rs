//! Two links over a reliable channel that reorders packets within the
//! allowed window (as EATT may): whatever the schedule, each side receives
//! exactly the bytes the other sent, in order, and no protocol error occurs.
//! Nothing overtakes a HELLO: a link ignores packets that arrive before the
//! peer's HELLO, and v1 tolerates reordering only, it assumes one bearer.
#![no_main]

use arbitrary::Arbitrary;
use blessh_core::link::{Event, Link};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Step {
    /// Side 0 or 1 writes data.
    Write(bool, Vec<u8>),
    /// Side 0 or 1 consumes everything it received so far.
    Consume(bool),
    /// Side 0 or 1 queues all its frames on the wire.
    Transmit(bool),
    /// Deliver one of the first 8 packets in flight towards side 0 or 1,
    /// as long as it is at most 60 sequence numbers ahead of the oldest one
    /// (the protocol's reorder window is 64); otherwise the oldest.
    Deliver(bool, u8),
}

#[derive(Arbitrary, Debug)]
struct Input {
    mtu: [u16; 2],
    window: [u16; 2],
    steps: Vec<Step>,
}

fuzz_target!(|input: Input| {
    let mut links = [
        Link::new(input.mtu[0] as usize, input.window[0] as u32 + 1),
        Link::new(input.mtu[1] as usize, input.window[1] as u32 + 1),
    ];
    let mut wire: [Vec<Vec<u8>>; 2] = [Vec::new(), Vec::new()]; // packets towards side i
    let mut sent: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut received: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    let mut unconsumed = [0usize; 2];

    let seq = |p: &[u8]| u16::from_le_bytes([p[1], p[2]]);
    let deliver = |links: &mut [Link; 2], wire: &mut [Vec<Vec<u8>>; 2], to: usize, n: usize, received: &mut [Vec<u8>; 2], unconsumed: &mut [usize; 2]| {
        if wire[to].is_empty() {
            return;
        }
        let mut n = n.min(wire[to].len() - 1);
        if !links[to].is_open() || seq(&wire[to][n]).wrapping_sub(seq(&wire[to][0])) > 60 {
            n = 0;
        }
        let pkt = wire[to].remove(n);
        let events = links[to].on_packet(&pkt).expect("protocol error between two correct links");
        for ev in events {
            if let Event::Data(d) = ev {
                unconsumed[to] += d.len();
                received[to].extend(d);
            }
        }
        links[to].check_invariants();
    };

    for step in input.steps {
        match step {
            Step::Write(side, d) => {
                let s = side as usize;
                links[s].send(&d);
                sent[s].extend(d);
            }
            Step::Consume(side) => {
                let s = side as usize;
                links[s].consumed(std::mem::take(&mut unconsumed[s]));
            }
            Step::Transmit(side) => {
                let s = side as usize;
                while let Some(p) = links[s].poll_transmit() {
                    wire[1 - s].push(p);
                }
            }
            Step::Deliver(to, n) => deliver(&mut links, &mut wire, to as usize, (n % 8) as usize, &mut received, &mut unconsumed),
        }
        // Received data is always a prefix of what was sent.
        for s in 0..2 {
            assert!(sent[1 - s].starts_with(&received[s]), "stream corrupted");
        }
    }

    // Drain: the application keeps consuming and both sides keep sending
    // until nothing moves; then everything sent must have arrived.
    for _ in 0..100_000 {
        let mut moved = false;
        for s in 0..2 {
            links[s].consumed(std::mem::take(&mut unconsumed[s]));
            while let Some(p) = links[s].poll_transmit() {
                wire[1 - s].push(p);
                moved = true;
            }
        }
        for to in 0..2 {
            while !wire[to].is_empty() {
                deliver(&mut links, &mut wire, to, 0, &mut received, &mut unconsumed);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    for s in 0..2 {
        assert_eq!(received[s], sent[1 - s], "data lost");
    }
});
