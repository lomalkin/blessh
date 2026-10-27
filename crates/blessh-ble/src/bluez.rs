//! BlueZ specifics shared by the server and the client: BleStream packet
//! transports over the GATT characteristic sockets, and L2CAP
//! connection-oriented channels (LE CoC) through the kernel.

use std::{io, sync::Arc, time::Duration};

use blessh_core::bridge::{PacketRx, PacketTx};
use bluer::{
    gatt::{CharacteristicReader, CharacteristicWriter},
    l2cap::{Security, SecurityLevel, Socket, SocketAddr, Stream, StreamListener},
    Address, AddressType,
};

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
        while stream.as_ref().send_mtu().is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, io::Error>(stream)
    };
    tokio::time::timeout(timeout, connect)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "L2CAP connect timed out"))?
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
