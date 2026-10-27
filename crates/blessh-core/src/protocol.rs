//! BleStream v1 wire format. See `docs/protocol.md` for the normative description.

use uuid::Uuid;

/// Primary GATT service exposed by the server.
pub const SERVICE_UUID: Uuid = Uuid::from_u128(0x5b1e7e00_6d2c_4f4a_9a3e_2c1b0e8f4a10);
/// Read-only server information (`key=value` pairs separated by `;`).
pub const INFO_UUID: Uuid = Uuid::from_u128(0x5b1e7e01_6d2c_4f4a_9a3e_2c1b0e8f4a10);
/// Client to server stream (write / write-without-response).
pub const RX_UUID: Uuid = Uuid::from_u128(0x5b1e7e02_6d2c_4f4a_9a3e_2c1b0e8f4a10);
/// Server to client stream (notify).
pub const TX_UUID: Uuid = Uuid::from_u128(0x5b1e7e03_6d2c_4f4a_9a3e_2c1b0e8f4a10);
/// PSM of the server's L2CAP channel as a little-endian u16 (read). The UUID
/// is Apple's `CBUUIDL2CAPPSMCharacteristicString`, which iOS and macOS
/// clients look for; INFO carries the same value as `psm`.
pub const PSM_UUID: Uuid = Uuid::from_u128(0xabdd3056_28fa_441d_a470_55a75a52553a);

/// Prefix of the line a server sends on an L2CAP channel, before any SSH
/// banner, when it refuses the session (busy, session unavailable). RFC 4253
/// lets servers send such lines; ssh shows them with -v.
pub const REFUSAL_PREFIX: &str = "blessh: ";

pub const VERSION: u8 = 1;
pub const MAGIC: [u8; 4] = *b"BLTM";

/// Frame header: type (1 byte) + sequence number (u16 little endian).
pub const HEADER_LEN: usize = 3;
/// Smallest usable frame size (ATT MTU 23 minus the 3 byte ATT header).
pub const MIN_FRAME: usize = 20;
/// Upper bound for a frame. Below the 512 byte ATT value limit because
/// BlueZ as GATT client drops writes without response above ~510 bytes
/// (seen with BlueZ 5.66 and EATT enabled).
pub const MAX_FRAME: usize = 500;

pub const T_HELLO: u8 = 0x01;
pub const T_DATA: u8 = 0x02;
pub const T_CREDIT: u8 = 0x03;
pub const T_CLOSE: u8 = 0x04;

const HELLO_LEN: usize = 10;

/// Reason codes carried by CLOSE frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CloseReason {
    Normal = 0,
    Busy = 1,
    ProtocolError = 2,
    SessionUnavailable = 3,
    Timeout = 4,
}

impl CloseReason {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Normal,
            1 => Self::Busy,
            2 => Self::ProtocolError,
            3 => Self::SessionUnavailable,
            4 => Self::Timeout,
            _ => Self::ProtocolError,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Hello {
        version: u8,
        flags: u8,
        window: u32,
    },
    Data(Vec<u8>),
    Credit(u32),
    Close {
        reason: CloseReason,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    UnknownType(u8),
    BadHello,
    BadCredit,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "frame shorter than header"),
            Self::UnknownType(t) => write!(f, "unknown frame type 0x{t:02x}"),
            Self::BadHello => write!(f, "malformed HELLO"),
            Self::BadCredit => write!(f, "malformed CREDIT"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl Frame {
    pub fn encode(&self, seq: u16) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload_len());
        let ty = match self {
            Frame::Hello { .. } => T_HELLO,
            Frame::Data(_) => T_DATA,
            Frame::Credit(_) => T_CREDIT,
            Frame::Close { .. } => T_CLOSE,
        };
        out.push(ty);
        out.extend_from_slice(&seq.to_le_bytes());
        match self {
            Frame::Hello {
                version,
                flags,
                window,
            } => {
                out.extend_from_slice(&MAGIC);
                out.push(*version);
                out.push(*flags);
                out.extend_from_slice(&window.to_le_bytes());
            }
            Frame::Data(d) => out.extend_from_slice(d),
            Frame::Credit(n) => out.extend_from_slice(&n.to_le_bytes()),
            Frame::Close { reason, message } => {
                out.push(*reason as u8);
                out.extend_from_slice(message.as_bytes());
            }
        }
        out
    }

    fn payload_len(&self) -> usize {
        match self {
            Frame::Hello { .. } => HELLO_LEN,
            Frame::Data(d) => d.len(),
            Frame::Credit(_) => 4,
            Frame::Close { message, .. } => 1 + message.len(),
        }
    }

    /// Decodes a frame, returning its sequence number and contents.
    pub fn decode(pkt: &[u8]) -> Result<(u16, Frame), DecodeError> {
        if pkt.len() < HEADER_LEN {
            return Err(DecodeError::TooShort);
        }
        let seq = u16::from_le_bytes([pkt[1], pkt[2]]);
        let p = &pkt[HEADER_LEN..];
        let frame = match pkt[0] {
            T_HELLO => {
                if p.len() < HELLO_LEN || p[..4] != MAGIC {
                    return Err(DecodeError::BadHello);
                }
                Frame::Hello {
                    version: p[4],
                    flags: p[5],
                    window: u32::from_le_bytes([p[6], p[7], p[8], p[9]]),
                }
            }
            T_DATA => Frame::Data(p.to_vec()),
            T_CREDIT => {
                if p.len() != 4 {
                    return Err(DecodeError::BadCredit);
                }
                Frame::Credit(u32::from_le_bytes([p[0], p[1], p[2], p[3]]))
            }
            T_CLOSE => {
                let reason = p
                    .first()
                    .map(|r| CloseReason::from_u8(*r))
                    .unwrap_or(CloseReason::Normal);
                let message = p.get(1..).map(close_text).unwrap_or_default();
                Frame::Close { reason, message }
            }
            t => return Err(DecodeError::UnknownType(t)),
        };
        Ok((seq, frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let frames = [
            Frame::Hello {
                version: 1,
                flags: 0,
                window: 16384,
            },
            Frame::Data(b"hello".to_vec()),
            Frame::Data(Vec::new()),
            Frame::Credit(4096),
            Frame::Close {
                reason: CloseReason::Busy,
                message: "busy".into(),
            },
        ];
        for (i, f) in frames.iter().enumerate() {
            let enc = f.encode(i as u16 * 1000);
            let (seq, dec) = Frame::decode(&enc).unwrap();
            assert_eq!(seq, i as u16 * 1000);
            assert_eq!(&dec, f);
        }
    }

    #[test]
    fn test_vectors() {
        // Shared with the Android implementation (BleStreamTest.kt).
        assert_eq!(
            Frame::Hello {
                version: 1,
                flags: 0,
                window: 0x4000
            }
            .encode(0),
            [0x01, 0x00, 0x00, b'B', b'L', b'T', b'M', 0x01, 0x00, 0x00, 0x40, 0x00, 0x00]
        );
        assert_eq!(
            Frame::Data(vec![0x61, 0x62]).encode(0x0102),
            [0x02, 0x02, 0x01, 0x61, 0x62]
        );
        assert_eq!(
            Frame::Credit(0x01020304).encode(7),
            [0x03, 0x07, 0x00, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(
            Frame::Close {
                reason: CloseReason::Normal,
                message: "ok".into()
            }
            .encode(9),
            [0x04, 0x09, 0x00, 0x00, b'o', b'k']
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Frame::decode(&[1, 2]), Err(DecodeError::TooShort));
        assert_eq!(Frame::decode(&[9, 0, 0]), Err(DecodeError::UnknownType(9)));
        assert_eq!(
            Frame::decode(&[1, 0, 0, b'X', b'L', b'T', b'M', 1, 0, 0, 0, 0, 0]),
            Err(DecodeError::BadHello)
        );
        assert_eq!(Frame::decode(&[3, 0, 0, 1]), Err(DecodeError::BadCredit));
    }
}

/// CLOSE message bytes as text; invalid UTF-8 is replaced.
#[cfg(not(kani))]
fn close_text(m: &[u8]) -> String {
    String::from_utf8_lossy(m).into_owned()
}

/// Model checking treats the CLOSE text as opaque: std's UTF-8 decoder is
/// trusted, and unrolling it for symbolic bytes exhausts the solver.
#[cfg(kani)]
fn close_text(_m: &[u8]) -> String {
    String::new()
}

/// Makes text that came from the peer (CLOSE messages) safe to log: control
/// characters are escaped, so it cannot start new journal entries or set
/// their priority, and the length is bounded.
pub fn printable(s: &str) -> String {
    let mut out: String = s.chars().take(200).flat_map(char::escape_debug).collect();
    if s.chars().count() > 200 {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod printable_tests {
    use super::printable;

    #[test]
    fn escapes_control_characters() {
        assert_eq!(printable("ok"), "ok");
        assert_eq!(printable("a\n<3>fake entry"), "a\\n<3>fake entry");
        assert_eq!(printable("\u{1b}[2J"), "\\u{1b}[2J");
        assert_eq!(printable(&"x".repeat(300)).len(), 203);
    }
}

/// Value of the INFO characteristic. `mtu` is the ATT MTU of the bearer the
/// read arrived on, for clients that cannot query it; `busy` says that
/// another device holds the GATT stream; `psm` is the server's L2CAP
/// channel, if it has one.
pub fn info(name: &str, mtu: u16, busy: bool, psm: Option<u16>) -> String {
    let busy = if busy { ";busy=1" } else { "" };
    let psm = psm.map(|p| format!(";psm={p}")).unwrap_or_default();
    format!("proto={VERSION};name={name};mtu={mtu}{busy}{psm}")
}

/// Value of a key in an INFO value.
pub fn info_value<'a>(info: &'a str, key: &str) -> Option<&'a str> {
    info.split(';')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// Whether an INFO value says another client holds the GATT stream.
pub fn info_busy(info: &str) -> bool {
    info_value(info, "busy") == Some("1")
}

/// The server's L2CAP PSM from an INFO value, if it offers a channel.
pub fn info_psm(info: &str) -> Option<u16> {
    info_value(info, "psm")?.parse().ok().filter(|p| *p > 0)
}

/// The line that refuses a session on an L2CAP channel.
pub fn refusal_line(reason: &str) -> String {
    format!("{REFUSAL_PREFIX}{reason}\r\n")
}

/// The part of an attribute value a read at `offset` returns (ATT Read Blob
/// continues a value longer than the MTU); `None` if the offset is past the
/// end.
pub fn read_at(value: &[u8], offset: u16) -> Option<&[u8]> {
    value.get(usize::from(offset)..)
}

#[cfg(test)]
mod info_tests {
    use super::{info, info_busy, info_psm, read_at};

    #[test]
    fn info_value() {
        assert_eq!(info("pi", 517, false, None), "proto=1;name=pi;mtu=517");
        assert_eq!(info("pi", 23, true, None), "proto=1;name=pi;mtu=23;busy=1");
        assert_eq!(
            info("pi", 23, false, Some(0x81)),
            "proto=1;name=pi;mtu=23;psm=129"
        );
    }

    #[test]
    fn info_busy_flag() {
        assert!(info_busy("proto=1;name=pi;mtu=23;busy=1"));
        assert!(!info_busy("proto=1;name=pi;mtu=23"));
        assert!(!info_busy("proto=1;name=busy=1x;mtu=23"));
    }

    #[test]
    fn info_psm_value() {
        assert_eq!(info_psm("proto=1;name=pi;mtu=23;psm=129"), Some(129));
        assert_eq!(info_psm("proto=1;name=pi;mtu=23"), None);
        assert_eq!(info_psm("proto=1;name=pi;mtu=23;psm=0"), None);
        assert_eq!(info_psm("proto=1;name=pi;mtu=23;psm=x"), None);
        assert_eq!(
            super::refusal_line("busy (4 sessions)"),
            "blessh: busy (4 sessions)\r\n"
        );
    }

    #[test]
    fn long_reads_continue_at_offset() {
        let v = info("a-rather-long-host-name", 23, false, None).into_bytes();
        let mut got = Vec::new();
        while let Some(part) = read_at(&v, got.len() as u16).filter(|p| !p.is_empty()) {
            got.extend_from_slice(&part[..part.len().min(22)]);
        }
        assert_eq!(got, v);
        assert_eq!(read_at(&v, v.len() as u16), Some(&[][..]));
        assert_eq!(read_at(&v, v.len() as u16 + 1), None);
    }
}
