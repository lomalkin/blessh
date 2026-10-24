//! Sans-IO state machine for one BleStream link (shared by server and client).
//!
//! The link turns a byte stream into frames that fit one ATT value, enforces
//! credit based flow control in both directions and restores frame order when
//! the GATT layer delivers packets out of order (possible with EATT).

use std::collections::{BTreeMap, VecDeque};

use crate::protocol::{CloseReason, DecodeError, Frame, HEADER_LEN, MAX_FRAME, MIN_FRAME, VERSION};

/// Frames that may arrive ahead of the next expected sequence number.
const REORDER_LIMIT: u16 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Peer completed the handshake.
    Open { window: u32 },
    /// Stream data from the peer, in order.
    Data(Vec<u8>),
    /// Peer closed the link.
    Closed {
        reason: CloseReason,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Decode(DecodeError),
    UnsupportedVersion(u8),
    UnexpectedHello,
    DataBeforeHello,
    WindowExceeded,
    OutOfOrder { expected: u16, got: u16 },
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "{e}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            Self::UnexpectedHello => write!(f, "duplicate HELLO"),
            Self::DataBeforeHello => write!(f, "frame received before HELLO"),
            Self::WindowExceeded => write!(f, "peer exceeded the receive window"),
            Self::OutOfOrder { expected, got } => {
                write!(f, "sequence error: expected {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for LinkError {}

impl From<DecodeError> for LinkError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

pub struct Link {
    max_frame: usize,
    tx_seq: u16,
    rx_seq: u16,
    reorder: BTreeMap<u16, Vec<u8>>,
    peer_open: bool,
    /// DATA bytes we are still allowed to send.
    send_credit: u64,
    /// DATA bytes the peer is still allowed to send us.
    recv_allowance: u64,
    recv_window: u32,
    /// Bytes handed to the application and not yet returned as credit.
    consumed: u32,
    control: VecDeque<Frame>,
    outgoing: VecDeque<u8>,
    closing: bool,
}

impl Link {
    /// Creates a link that sends frames of at most `max_frame` bytes and
    /// allows the peer to have `recv_window` unconsumed bytes in flight.
    pub fn new(max_frame: usize, recv_window: u32) -> Self {
        let max_frame = max_frame.clamp(MIN_FRAME, MAX_FRAME);
        let mut control = VecDeque::new();
        control.push_back(Frame::Hello {
            version: VERSION,
            flags: 0,
            window: recv_window,
        });
        Self {
            max_frame,
            tx_seq: 0,
            rx_seq: 0,
            reorder: BTreeMap::new(),
            peer_open: false,
            send_credit: 0,
            recv_allowance: recv_window as u64,
            recv_window,
            consumed: 0,
            control,
            outgoing: VecDeque::new(),
            closing: false,
        }
    }

    pub fn max_frame(&self) -> usize {
        self.max_frame
    }

    pub fn is_open(&self) -> bool {
        self.peer_open
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// Bytes queued for sending but not yet framed.
    pub fn pending_out(&self) -> usize {
        self.outgoing.len()
    }

    /// True when nothing is left to transmit.
    pub fn is_flushed(&self) -> bool {
        self.control.is_empty() && (self.outgoing.is_empty() || self.closing)
    }

    /// Queues stream data for sending.
    pub fn send(&mut self, data: &[u8]) {
        if !self.closing {
            self.outgoing.extend(data);
        }
    }

    /// Queues a CLOSE frame; no further data is sent after it.
    pub fn close(&mut self, reason: CloseReason, message: &str) {
        if !self.closing {
            self.closing = true;
            self.outgoing.clear();
            let mut end = message.len().min(self.max_frame - HEADER_LEN - 1);
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            self.control.push_back(Frame::Close {
                reason,
                message: message[..end].to_owned(),
            });
        }
    }

    /// Reports that the application consumed `n` bytes of received data,
    /// so the space can be granted back to the peer.
    pub fn consumed(&mut self, n: usize) {
        self.consumed = self.consumed.saturating_add(n as u32);
        let threshold = (self.recv_window / 4).max(1);
        if self.consumed >= threshold && !self.closing {
            let n = std::mem::take(&mut self.consumed);
            self.recv_allowance += n as u64;
            self.control.push_back(Frame::Credit(n));
        }
    }

    /// Returns the next frame to transmit, if any.
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        let frame = if let Some(f) = self.control.pop_front() {
            f
        } else if self.peer_open
            && !self.closing
            && !self.outgoing.is_empty()
            && self.send_credit > 0
        {
            let n = self
                .outgoing
                .len()
                .min(self.max_frame - HEADER_LEN)
                .min(self.send_credit as usize);
            self.send_credit -= n as u64;
            Frame::Data(self.outgoing.drain(..n).collect())
        } else {
            return None;
        };
        let seq = self.tx_seq;
        self.tx_seq = self.tx_seq.wrapping_add(1);
        Some(frame.encode(seq))
    }

    /// Processes one received packet.
    pub fn on_packet(&mut self, pkt: &[u8]) -> Result<Vec<Event>, LinkError> {
        let (seq, frame) = Frame::decode(pkt)?;
        let mut events = Vec::new();
        if seq != self.rx_seq {
            let ahead = seq.wrapping_sub(self.rx_seq);
            if ahead == 0 || ahead > REORDER_LIMIT || self.reorder.contains_key(&seq) {
                return Err(LinkError::OutOfOrder {
                    expected: self.rx_seq,
                    got: seq,
                });
            }
            self.reorder.insert(seq, pkt.to_vec());
            return Ok(events);
        }
        self.rx_seq = self.rx_seq.wrapping_add(1);
        self.handle(frame, &mut events)?;
        while let Some(pkt) = self.reorder.remove(&self.rx_seq) {
            self.rx_seq = self.rx_seq.wrapping_add(1);
            let (_, frame) = Frame::decode(&pkt)?;
            self.handle(frame, &mut events)?;
        }
        Ok(events)
    }

    fn handle(&mut self, frame: Frame, events: &mut Vec<Event>) -> Result<(), LinkError> {
        match frame {
            Frame::Hello {
                version, window, ..
            } => {
                if self.peer_open {
                    return Err(LinkError::UnexpectedHello);
                }
                if version != VERSION {
                    return Err(LinkError::UnsupportedVersion(version));
                }
                self.peer_open = true;
                self.send_credit = window as u64;
                events.push(Event::Open { window });
            }
            Frame::Close { reason, message } => events.push(Event::Closed { reason, message }),
            _ if !self.peer_open => return Err(LinkError::DataBeforeHello),
            Frame::Data(d) => {
                if d.len() as u64 > self.recv_allowance {
                    return Err(LinkError::WindowExceeded);
                }
                self.recv_allowance -= d.len() as u64;
                if !d.is_empty() {
                    events.push(Event::Data(d));
                }
            }
            Frame::Credit(n) => self.send_credit += n as u64,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump(from: &mut Link, to: &mut Link) -> Vec<Event> {
        let mut ev = Vec::new();
        while let Some(p) = from.poll_transmit() {
            assert!(p.len() <= from.max_frame());
            ev.extend(to.on_packet(&p).unwrap());
        }
        ev
    }

    fn data(ev: &[Event]) -> Vec<u8> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Data(d) => Some(d.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn handshake_and_transfer() {
        let mut a = Link::new(20, 100);
        let mut b = Link::new(64, 1000);
        assert_eq!(pump(&mut a, &mut b), vec![Event::Open { window: 100 }]);
        assert_eq!(pump(&mut b, &mut a), vec![Event::Open { window: 1000 }]);
        a.send(&[7u8; 300]);
        let ev = pump(&mut a, &mut b);
        assert_eq!(data(&ev), vec![7u8; 300]);
        // b -> a limited by a's window of 100 bytes.
        b.send(&[1u8; 250]);
        let ev = pump(&mut b, &mut a);
        assert_eq!(data(&ev).len(), 100);
        assert_eq!(b.pending_out(), 150);
        a.consumed(100);
        pump(&mut a, &mut b);
        let ev = pump(&mut b, &mut a);
        assert_eq!(data(&ev).len(), 100);
        a.consumed(100);
        pump(&mut a, &mut b);
        assert_eq!(data(&pump(&mut b, &mut a)).len(), 50);
    }

    #[test]
    fn data_waits_for_hello() {
        let mut a = Link::new(20, 100);
        let mut b = Link::new(20, 100);
        a.send(b"early");
        let hello = a.poll_transmit().unwrap();
        assert!(a.poll_transmit().is_none(), "no data before peer HELLO");
        b.on_packet(&hello).unwrap();
        pump(&mut b, &mut a);
        assert_eq!(data(&pump(&mut a, &mut b)), b"early");
    }

    #[test]
    fn reorders() {
        let mut a = Link::new(20, 10_000);
        let mut b = Link::new(20, 10_000);
        pump(&mut a, &mut b);
        pump(&mut b, &mut a);
        a.send(&(0..=255u8).cycle().take(200).collect::<Vec<_>>());
        let mut pkts = Vec::new();
        while let Some(p) = a.poll_transmit() {
            pkts.push(p);
        }
        pkts.swap(0, 1);
        pkts.swap(3, 6);
        let mut out = Vec::new();
        for p in pkts {
            for e in b.on_packet(&p).unwrap() {
                if let Event::Data(d) = e {
                    out.extend(d);
                }
            }
        }
        assert_eq!(out, (0..=255u8).cycle().take(200).collect::<Vec<_>>());
    }

    #[test]
    fn detects_violations() {
        let mut a = Link::new(20, 100);
        let mut b = Link::new(20, 10);
        pump(&mut a, &mut b);
        pump(&mut b, &mut a);
        // Forge a DATA frame that ignores the 10 byte window.
        let pkt = Frame::Data(vec![0; 11]).encode(1);
        assert_eq!(b.on_packet(&pkt), Err(LinkError::WindowExceeded));
        let mut c = Link::new(20, 100);
        assert_eq!(
            c.on_packet(&Frame::Data(vec![1]).encode(0)),
            Err(LinkError::DataBeforeHello)
        );
        let mut d = Link::new(20, 100);
        assert!(matches!(
            d.on_packet(&Frame::Credit(1).encode(500)),
            Err(LinkError::OutOfOrder { .. })
        ));
    }

    #[test]
    fn close_discards_pending() {
        let mut a = Link::new(20, 100);
        let mut b = Link::new(20, 100);
        pump(&mut a, &mut b);
        pump(&mut b, &mut a);
        a.send(&[0; 1000]);
        a.close(CloseReason::Normal, "bye");
        let ev = pump(&mut a, &mut b);
        assert_eq!(
            ev,
            vec![Event::Closed {
                reason: CloseReason::Normal,
                message: "bye".into()
            }]
        );
        assert!(a.is_flushed());
    }

    #[test]
    fn close_message_fits_frame() {
        let mut a = Link::new(20, 100);
        a.poll_transmit();
        a.close(CloseReason::ProtocolError, &"é".repeat(40));
        let pkt = a.poll_transmit().unwrap();
        assert!(pkt.len() <= 20);
        let (_, f) = Frame::decode(&pkt).unwrap();
        assert!(matches!(f, Frame::Close { message, .. } if message == "é".repeat(8)));
    }

    #[test]
    fn sequence_wraps() {
        let mut a = Link::new(20, u32::MAX);
        let mut b = Link::new(20, u32::MAX);
        pump(&mut a, &mut b);
        pump(&mut b, &mut a);
        a.send(&vec![5u8; 17 * 70_000]);
        assert_eq!(data(&pump(&mut a, &mut b)).len(), 17 * 70_000);
    }
}
