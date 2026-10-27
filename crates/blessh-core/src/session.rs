//! Session preamble: one text line the daemon writes to the session socket
//! before the SSH stream, so that `blessh-session` can log which BLE device
//! the session belongs to before it starts `sshd -i`.
//!
//! ```text
//! BLESSH-SESSION/1 id=<16 hex> device=<AA:BB:CC:DD:EE:FF> type=<public|random|unknown>\n
//! ```
//!
//! The socket is only accessible to the daemon's user, so the line is trusted
//! as far as the daemon is; it is still validated strictly.

use std::fmt;

pub const MAGIC: &str = "BLESSH-SESSION/1";
/// Upper bound for the line, including the newline.
pub const MAX_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preamble {
    pub id: String,
    pub device: String,
    pub address_type: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(pub &'static str);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid session preamble: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Random 64-bit session id as 16 hex digits.
pub fn new_id() -> String {
    use std::io::Read;
    let mut b = [0u8; 8];
    let random = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    if random.is_err() {
        // Fall back to time and pid; ids only need to be unique in the logs.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        b = (t.as_nanos() as u64 ^ (std::process::id() as u64) << 32).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Preamble {
    /// The line to send, including the trailing newline.
    pub fn encode(&self) -> String {
        format!(
            "{MAGIC} id={} device={} type={}\n",
            self.id, self.device, self.address_type
        )
    }

    /// Parses a line without its trailing newline.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        let mut parts = line.split(' ');
        if parts.next() != Some(MAGIC) {
            return Err(ParseError("bad magic"));
        }
        let mut field = |key: &str| {
            parts
                .next()
                .and_then(|p| p.strip_prefix(key))
                .and_then(|p| p.strip_prefix('='))
                .ok_or(ParseError("missing field"))
        };
        let id = field("id")?;
        let device = field("device")?;
        let address_type = field("type")?;
        if parts.next().is_some() {
            return Err(ParseError("trailing data"));
        }
        if id.len() != 16 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(ParseError("bad id"));
        }
        let octets: Vec<&str> = device.split(':').collect();
        if octets.len() != 6
            || !octets
                .iter()
                .all(|o| o.len() == 2 && o.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(ParseError("bad device address"));
        }
        if !matches!(address_type, "public" | "random" | "unknown") {
            return Err(ParseError("bad address type"));
        }
        Ok(Self {
            id: id.into(),
            device: device.into(),
            address_type: address_type.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = Preamble {
            id: new_id(),
            device: "B8:27:EB:DF:42:54".into(),
            address_type: "random".into(),
        };
        let line = p.encode();
        assert!(line.len() <= MAX_LEN);
        assert_eq!(Preamble::parse(line.trim_end_matches('\n')), Ok(p));
    }

    #[test]
    fn rejects_malformed() {
        let ok = "BLESSH-SESSION/1 id=0123456789abcdef device=B8:27:EB:DF:42:54 type=public";
        assert!(Preamble::parse(ok).is_ok());
        for bad in [
            "",
            "SSH-2.0-OpenSSH_9.2",
            "BLESSH-SESSION/2 id=0123456789abcdef device=B8:27:EB:DF:42:54 type=public",
            "BLESSH-SESSION/1 id=0123 device=B8:27:EB:DF:42:54 type=public",
            "BLESSH-SESSION/1 id=0123456789abcdef device=B8:27:EB:DF:42 type=public",
            "BLESSH-SESSION/1 id=0123456789abcdef device=B8:27:EB:DF:42:54 type=x",
            "BLESSH-SESSION/1 id=0123456789abcdef device=B8:27:EB:DF:42:54 type=public extra",
            "BLESSH-SESSION/1 device=B8:27:EB:DF:42:54 id=0123456789abcdef type=public",
        ] {
            assert!(Preamble::parse(bad).is_err(), "{bad}");
        }
    }
}
