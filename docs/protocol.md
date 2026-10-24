# BleStream v1

BleStream carries one reliable, ordered, flow-controlled byte stream in each
direction over a pair of GATT characteristics. ble-term runs SSH over it: the
client's SSH library talks to a per-connection `sshd -i` on the server.

## GATT layout

| Item | UUID | Properties |
|---|---|---|
| Service | `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10` | primary, advertised |
| INFO | `5b1e7e01-6d2c-4f4a-9a3e-2c1b0e8f4a10` | read: UTF-8 `proto=1;name=<host>;mtu=<n>` |
| RX (client → server) | `5b1e7e02-6d2c-4f4a-9a3e-2c1b0e8f4a10` | write, write without response |
| TX (server → client) | `5b1e7e03-6d2c-4f4a-9a3e-2c1b0e8f4a10` | notify |

INFO is `key=value` pairs separated by `;`; clients ignore unknown keys.
`mtu` (servers from 0.1.3) is the ATT MTU of the bearer the read arrived on,
for clients that cannot query it; read INFO after the MTU exchange. The
value can be longer than one ATT PDU; the server serves long reads (Read
Blob) from the requested offset.

No characteristic requires encryption or pairing: confidentiality and
authentication come from SSH (host key pinning on the client, PAM / public
keys on the server).

Every ATT write to RX and every notification on TX carries exactly one frame.
A frame never exceeds min(ATT_MTU − 3, 500) bytes. The 500 byte cap leaves
room below the 512 byte attribute limit: BlueZ acting as GATT client was seen
to drop larger writes without response when EATT is enabled.

The client writes RX with write without response or with write request
(write with response); the server treats both alike. Either way a frame
fits in one ATT PDU: long writes (Prepare/Execute Write), which split a
value across PDUs, are not allowed.

BleStream relies on the link layer for reliable, ordered delivery and
assumes a single ATT bearer: the connection's unenhanced ATT bearer. With
several bearers (EATT), frames can overtake each other by any number of
positions, so v1 does not support EATT; both sides should have it off
(BlueZ: `Channels = 1` in the `[GATT]` section of `main.conf`, the default
since 5.67; phones open EATT only on encrypted links, and ble-term links
are not encrypted).

## Frames

```
offset 0   u8      type
offset 1   u16 LE  sequence number
offset 3   ...     payload
```

Each direction numbers its frames 0, 1, 2, … (wrapping at 65536); every frame
type consumes a number. The receiver processes frames strictly in sequence
order. As a tolerance, not a guarantee, a frame that arrives early (up to 64
ahead) is held until the gap is filled; anything else is a protocol error.
Implementations log a warning on the first early frame, since on a single
bearer it does not happen.

| Type | Name | Payload |
|---|---|---|
| `0x01` | HELLO | `"BLTM"` (4 bytes), version u8 (= 1), flags u8 (= 0), window u32 LE |
| `0x02` | DATA | stream bytes (may be empty) |
| `0x03` | CREDIT | u32 LE: additional DATA bytes the peer may send |
| `0x04` | CLOSE | reason u8, optional UTF-8 message |

HELLO flags are sent as 0 and receivers ignore unknown bits; receivers also
ignore bytes after the window field. Both are reserved for compatible
extensions, see [compatibility.md](compatibility.md).

CLOSE reasons: 0 normal, 1 busy (another client is connected), 2 protocol
error, 3 session unavailable (server could not start sshd), 4 timeout.

## Session

1. The client connects, subscribes to TX notifications and sends HELLO on RX
   (sequence 0). The server sends its HELLO on TX once both characteristics are
   in use. Both HELLOs may cross; neither side sends DATA before it has
   received the peer's HELLO.
2. Until the peer's HELLO has arrived, a side accepts only a HELLO or a
   CLOSE (e.g. busy) with sequence number 0 and silently ignores every
   other packet: on a reused connection, frames of the previous session (a
   late CLOSE, a CREDIT in flight) can still arrive. A second HELLO or an
   unsupported version closes the link with reason 2.
3. After the handshake the server starts the SSH session; the stream then
   carries raw SSH protocol bytes in both directions.
4. Either side ends the session with CLOSE and the server then drops the BLE
   connection. A BLE disconnect without CLOSE ends the session as well; the
   server never resumes a session on a new connection.

The server allows the handshake 10 s (default) after the client starts using
the characteristics.

## Flow control

Each side announces in HELLO how many DATA payload bytes it can buffer
(`window`). The sender keeps a credit counter: it starts at the peer's window,
each DATA frame subtracts its payload length, and each CREDIT frame adds its
value. DATA is only sent while the counter is positive and never exceeds it.

The receiver returns credit as the application consumes data (written to sshd
or handed to the SSH library), batching CREDIT frames once at least a quarter
of its window has been consumed. A peer that sends more than it was granted is
closed with reason 2.

Control frames (HELLO, CREDIT, CLOSE) are not subject to credit.

Rationale: BlueZ queues notifications without limit and drops writes without
response if the daemon falls behind, and mobile stacks offer no backpressure
for notifications. Credits bound every queue on the path; the link layer is
already reliable, so no retransmission is needed.

## Client rules

* Use write without response for RX and keep only the OS-permitted number of
  writes outstanding (Android: wait for `onCharacteristicWrite`; iOS: wait for
  `canSendWriteWithoutResponse`). Where writes without response can be lost
  silently (a lost frame ends the session), use write with response, one
  write at a time; uplink throughput is lower.
* Frame size = min(negotiated ATT MTU − 3, 500) (Android: request MTU 517
  first). A client that cannot query the MTU (Web Bluetooth) uses the `mtu`
  key of INFO, read after the MTU exchange. Without it, it starts at 20 and
  may raise the size to the longest TX notification received: one bearer
  has one MTU for both directions.
* Pass DATA bytes to the SSH layer unchanged; never decode text per packet.

## Test vectors

```
HELLO v1 window=0x4000, seq 0 : 01 00 00 42 4c 54 4d 01 00 00 40 00 00
DATA "ab", seq 0x0102         : 02 02 01 61 62
CREDIT 0x01020304, seq 7      : 03 07 00 04 03 02 01
CLOSE normal "ok", seq 9      : 04 09 00 00 6f 6b
```
