//! BlueZ specifics shared by the server and the client: BleStream packet
//! transports over the GATT characteristic sockets, L2CAP
//! connection-oriented channels (LE CoC) through the kernel, and a direct
//! LE link with a minimal ATT client for reading the server's INFO without
//! bluetoothd's GATT client.

use std::{io, os::fd::AsRawFd, sync::Arc, time::Duration};

use blessh_core::{
    bridge::{PacketRx, PacketTx},
    protocol::{INFO_UUID, SERVICE_UUID},
};
use bluer::{
    gatt::{CharacteristicReader, CharacteristicWriter},
    l2cap::{Security, SecurityLevel, SeqPacket, Socket, SocketAddr, Stream, StreamListener},
    Address, AddressType,
};
use log::debug;

/// Packets the peer writes to the RX characteristic (AcquireWrite socket).
pub struct GattRx(pub CharacteristicReader);

impl PacketRx for GattRx {
    async fn recv(&mut self) -> io::Result<Vec<u8>> {
        self.0.recv().await
    }
}

/// Notifications sent through the TX characteristic (AcquireNotify socket).
#[derive(Clone)]
pub struct GattTx(pub Arc<CharacteristicWriter>);

impl PacketTx for GattTx {
    async fn send(&mut self, pkt: &[u8]) -> io::Result<()> {
        self.0.send(pkt).await
    }
}

/// L2CAP channels are as unprotected as the GATT characteristics: SSH
/// provides confidentiality and authentication, so no pairing is needed.
fn no_security() -> Security {
    Security {
        level: SecurityLevel::Low,
        key_size: 0,
    }
}

/// Listens for LE connection-oriented channels on the adapter with address
/// `local`; the kernel assigns a dynamic PSM (`local_addr().psm`).
pub async fn l2cap_listen(
    local: Address,
    recv_mtu: u16,
    backlog: u32,
) -> io::Result<StreamListener> {
    let sock = Socket::<Stream>::new_stream()?;
    sock.set_security(no_security())?;
    sock.set_recv_mtu(recv_mtu)?;
    sock.bind(SocketAddr::new(local, AddressType::LePublic, 0))?;
    sock.listen(backlog)
}

/// Opens a channel to `peer` from the adapter with address `local` and waits
/// until it is connected: bluer's connect returns while the connection is
/// still in progress (SO_ERROR is 0 then as well), so readiness is checked
/// through the send MTU, which the kernel knows only once connected.
pub async fn l2cap_connect(
    local: Address,
    peer: Address,
    peer_type: AddressType,
    psm: u16,
    recv_mtu: u16,
    timeout: Duration,
) -> io::Result<Stream> {
    let sock = Socket::<Stream>::new_stream()?;
    sock.set_security(no_security())?;
    sock.set_recv_mtu(recv_mtu)?;
    sock.bind(SocketAddr::new(local, AddressType::LePublic, 0))?;
    let connect = async {
        let stream = sock.connect(SocketAddr::new(peer, peer_type, psm)).await?;
        wait_connected(stream.as_ref().as_raw_fd(), || {
            stream.as_ref().send_mtu().is_ok()
        })
        .await?;
        Ok::<_, io::Error>(stream)
    };
    tokio::time::timeout(timeout, connect)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "L2CAP connect timed out"))?
}

/// Waits until the socket is connected (`ready`) or the kernel has recorded
/// a failure in SO_ERROR (refused, no such device, timed out).
async fn wait_connected(fd: i32, ready: impl Fn() -> bool) -> io::Result<()> {
    loop {
        if ready() {
            return Ok(());
        }
        let mut err: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: the pointers are valid for the given length.
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut err as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        if r == 0 && err != 0 && err != libc::EINPROGRESS && err != libc::EAGAIN {
            return Err(io::Error::from_raw_os_error(err));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// ATT fixed channel identifier.
const ATT_CID: u16 = 4;
/// ATT MTU offered to the server; a read of INFO fits in one response.
const ATT_MTU: u16 = 517;

mod att {
    pub const ERROR_RSP: u8 = 0x01;
    pub const EXCHANGE_MTU_REQ: u8 = 0x02;
    pub const EXCHANGE_MTU_RSP: u8 = 0x03;
    pub const FIND_INFO_REQ: u8 = 0x04;
    pub const FIND_BY_TYPE_VALUE_REQ: u8 = 0x06;
    pub const READ_BY_TYPE_REQ: u8 = 0x08;
    pub const READ_REQ: u8 = 0x0a;
    pub const READ_BLOB_REQ: u8 = 0x0c;
    pub const READ_BY_GROUP_TYPE_REQ: u8 = 0x10;
    pub const INDICATION: u8 = 0x1d;
    pub const CONFIRMATION: u8 = 0x1e;
    pub const COMMAND_FLAG: u8 = 0x40;

    pub const E_INVALID_HANDLE: u8 = 0x01;
    pub const E_REQUEST_NOT_SUPPORTED: u8 = 0x06;
    pub const E_INVALID_OFFSET: u8 = 0x07;
    pub const E_ATTRIBUTE_NOT_FOUND: u8 = 0x0a;
    pub const E_ATTRIBUTE_NOT_LONG: u8 = 0x0b;

    pub const PRIMARY_SERVICE: u16 = 0x2800;
}

/// Why a request to the server failed.
#[derive(Debug)]
pub enum AttError {
    /// The server answered with an ATT error code.
    Response(u8),
    /// The link or the socket failed.
    Io(io::Error),
}

impl From<io::Error> for AttError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for AttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Response(e) => write!(f, "ATT error 0x{e:02x}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// An LE link to a server that this process made through the kernel, with
/// the ATT fixed channel held by this process.
///
/// bluetoothd attaches no ATT channel to such a link (a connecting fixed
/// channel takes precedence over its listener), so it neither discovers the
/// server's services nor probes them with its profiles, which on an
/// unpaired link ends in a pairing request and a lost link (BLE MIDI from
/// PipeWire, for example). It also connects over LE for sure: bluetoothd
/// prefers BR/EDR for a dual-mode device such as a Mac. The server's
/// bluetoothd still discovers this side and gets "no attributes".
///
/// The link stays up while the ATT socket is open; `serve` keeps answering
/// the server during a session.
pub struct AttLink {
    sock: SeqPacket,
    peer: Address,
    mtu: usize,
}

impl AttLink {
    /// Connects to `peer` over LE from the adapter with address `local`.
    /// `EBUSY` means that this machine already holds an ATT channel to the
    /// device: bluetoothd (the device is connected through it) or another
    /// blessh.
    pub async fn connect(
        local: Address,
        peer: Address,
        peer_type: AddressType,
        timeout: Duration,
    ) -> io::Result<Self> {
        let sock = Socket::<SeqPacket>::new_seq_packet()?;
        sock.set_security(no_security())?;
        // A fixed channel is bound to its CID on both ends.
        sock.bind(SocketAddr {
            addr: local,
            addr_type: AddressType::LePublic,
            psm: 0,
            cid: ATT_CID,
        })?;
        let sa = SocketAddr {
            addr: peer,
            addr_type: peer_type,
            psm: 0,
            cid: ATT_CID,
        };
        let connect = async {
            let sock = sock.connect(sa).await?;
            wait_connected(sock.as_ref().as_raw_fd(), || sock.send_mtu().is_ok()).await?;
            Ok::<_, io::Error>(sock)
        };
        let sock = tokio::time::timeout(timeout, connect)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "LE connect timed out"))??;
        let mut link = Self {
            sock,
            peer,
            mtu: 23,
        };
        match link
            .request(
                &[att::EXCHANGE_MTU_REQ, ATT_MTU as u8, (ATT_MTU >> 8) as u8],
                timeout,
            )
            .await
        {
            Ok(rsp) if rsp.len() >= 3 => {
                let theirs = u16::from_le_bytes([rsp[1], rsp[2]]).max(23);
                link.mtu = theirs.min(ATT_MTU) as usize;
            }
            Ok(_) => {}
            Err(AttError::Response(e)) => {
                debug!("{peer}: MTU exchange refused: ATT error 0x{e:02x}")
            }
            Err(AttError::Io(e)) => return Err(e),
        }
        Ok(link)
    }

    pub fn peer(&self) -> Address {
        self.peer
    }

    /// Reads the server's INFO characteristic: finds the service, then the
    /// characteristic, then reads the value (in blobs if needed).
    pub async fn read_info(&self, timeout: Duration) -> Result<String, AttError> {
        let mut req = vec![att::FIND_BY_TYPE_VALUE_REQ, 0x01, 0x00, 0xff, 0xff];
        req.extend(att::PRIMARY_SERVICE.to_le_bytes());
        req.extend(SERVICE_UUID.as_u128().to_le_bytes());
        let rsp = self.request(&req, timeout).await?;
        let (start, end) = match rsp.get(1..5) {
            Some(r) => (
                u16::from_le_bytes([r[0], r[1]]),
                u16::from_le_bytes([r[2], r[3]]),
            ),
            None => return Err(AttError::Response(att::E_ATTRIBUTE_NOT_FOUND)),
        };
        let mut req = vec![att::READ_BY_TYPE_REQ];
        req.extend(start.to_le_bytes());
        req.extend(end.to_le_bytes());
        req.extend(INFO_UUID.as_u128().to_le_bytes());
        let rsp = self.request(&req, timeout).await?;
        let handle = match rsp.get(2..4) {
            Some(h) => u16::from_le_bytes([h[0], h[1]]),
            None => return Err(AttError::Response(att::E_ATTRIBUTE_NOT_FOUND)),
        };
        let mut req = vec![att::READ_REQ];
        req.extend(handle.to_le_bytes());
        let mut value = self.request(&req, timeout).await?[1..].to_vec();
        // A value as long as the MTU allows may go on.
        while value.len() + 1 >= self.mtu && value.len() < 512 {
            let mut req = vec![att::READ_BLOB_REQ];
            req.extend(handle.to_le_bytes());
            req.extend((value.len() as u16).to_le_bytes());
            match self.request(&req, timeout).await {
                Ok(rsp) if rsp.len() > 1 => value.extend(&rsp[1..]),
                Ok(_) => break,
                Err(AttError::Response(att::E_INVALID_OFFSET | att::E_ATTRIBUTE_NOT_LONG)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(String::from_utf8_lossy(&value).into_owned())
    }

    /// Sends a request and waits for its response, answering the server's
    /// own requests meanwhile.
    async fn request(&self, req: &[u8], timeout: Duration) -> Result<Vec<u8>, AttError> {
        let opcode = req[0];
        self.sock.send(req).await?;
        let wait = async {
            let mut buf = vec![0u8; 1024];
            loop {
                let n = self.sock.recv(&mut buf).await?;
                let pdu = &buf[..n];
                match pdu.first() {
                    Some(&att::ERROR_RSP) if pdu.len() >= 5 && pdu[1] == opcode => {
                        return Err(AttError::Response(pdu[4]));
                    }
                    Some(&op) if op == opcode + 1 => return Ok(pdu.to_vec()),
                    Some(_) => self.answer(pdu).await?,
                    None => {}
                }
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no answer from the server"))?
    }

    /// Answers one PDU from the server as an ATT server without attributes.
    async fn answer(&self, pdu: &[u8]) -> io::Result<()> {
        let op = pdu[0];
        let handle = |at: usize| pdu.get(at..at + 2).map_or([0, 0], |h| [h[0], h[1]]);
        let rsp: Vec<u8> = match op {
            att::EXCHANGE_MTU_REQ => {
                vec![att::EXCHANGE_MTU_RSP, ATT_MTU as u8, (ATT_MTU >> 8) as u8]
            }
            att::FIND_INFO_REQ
            | att::FIND_BY_TYPE_VALUE_REQ
            | att::READ_BY_TYPE_REQ
            | att::READ_BY_GROUP_TYPE_REQ => {
                let h = handle(1);
                vec![att::ERROR_RSP, op, h[0], h[1], att::E_ATTRIBUTE_NOT_FOUND]
            }
            att::INDICATION => vec![att::CONFIRMATION],
            _ if op & att::COMMAND_FLAG != 0 || op & 1 == 1 => return Ok(()),
            att::READ_REQ | att::READ_BLOB_REQ | 0x12 | 0x16 | 0x18 => {
                let h = handle(1);
                vec![att::ERROR_RSP, op, h[0], h[1], att::E_INVALID_HANDLE]
            }
            _ => vec![att::ERROR_RSP, op, 0, 0, att::E_REQUEST_NOT_SUPPORTED],
        };
        self.sock.send(&rsp).await?;
        Ok(())
    }

    /// Keeps answering the server until the link ends; returns why.
    pub async fn serve(&self) -> io::Error {
        let mut buf = vec![0u8; 1024];
        loop {
            match self.sock.recv(&mut buf).await {
                Ok(0) => return io::ErrorKind::UnexpectedEof.into(),
                Ok(n) => {
                    if let Err(e) = self.answer(&buf[..n]).await {
                        return e;
                    }
                }
                Err(e) => return e,
            }
        }
    }
}

/// Whether an error on an L2CAP socket means that the channel or the link
/// ended (closed by the peer, lost) rather than a fault: the kernel reports
/// both as errors, not as EOF.
pub fn is_link_end(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(
            libc::ECONNRESET
                | libc::ECONNABORTED
                | libc::ETIMEDOUT
                | libc::ENOTCONN
                | libc::EPIPE
                | libc::ESHUTDOWN
        )
    ) || e.kind() == io::ErrorKind::UnexpectedEof
}

/// How a channel ended, for logs: the errno carries the HCI disconnect reason.
pub fn link_end_reason(e: &io::Error) -> &'static str {
    match e.raw_os_error() {
        Some(libc::ECONNRESET) => "peer closed the channel",
        Some(libc::ECONNABORTED) => "link closed on this side",
        Some(libc::ETIMEDOUT) => "link lost (supervision timeout)",
        Some(libc::ENOTCONN | libc::EPIPE | libc::ESHUTDOWN) => "channel already closed",
        _ => "channel error",
    }
}
