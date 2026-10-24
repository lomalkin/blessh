# BleStream v1

BleStream carries one reliable, ordered, flow-controlled byte stream in each
direction over a pair of GATT characteristics. ble-term runs SSH over it: the
client's SSH library talks to a per-connection `sshd -i` on the server.

## GATT layout

| Item | UUID | Properties |
|---|---|---|
| Service | `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10` | primary, advertised |
| INFO | `5b1e7e01-6d2c-4f4a-9a3e-2c1b0e8f4a10` | read: UTF-8 `proto=1;name=<host>` |
| RX (client → server) | `5b1e7e02-6d2c-4f4a-9a3e-2c1b0e8f4a10` | write, write without response |
| TX (server → client) | `5b1e7e03-6d2c-4f4a-9a3e-2c1b0e8f4a10` | notify |

No characteristic requires encryption or pairing: confidentiality and
authentication come from SSH (host key pinning on the client, PAM / public
keys on the server).

Every ATT write to RX and every notification on TX carries exactly one frame.
A frame never exceeds the ATT payload limit (ATT_MTU − 3; the server keeps a
few bytes of margin, see `max_frame` below).

## Frames

```
offset 0   u8      type
offset 1   u16 LE  sequence number
offset 3   ...     payload
```

Each direction numbers its frames 0, 1, 2, … (wrapping at 65536); every frame
type consumes a number. The receiver processes frames strictly in sequence
order. A frame that arrives early (up to 64 ahead, e.g. because GATT used
several EATT bearers) is held until the gap is filled; anything else is a
protocol error.

| Type | Name | Payload |
|---|---|---|
| `0x01` | HELLO | `"BLTM"` (4 bytes), version u8 (= 1), flags u8 (= 0), window u32 LE |
| `0x02` | DATA | stream bytes (may be empty) |
| `0x03` | CREDIT | u32 LE: additional DATA bytes the peer may send |
| `0x04` | CLOSE | reason u8, optional UTF-8 message |

CLOSE reasons: 0 normal, 1 busy (another client is connected), 2 protocol
error, 3 session unavailable (server could not start sshd), 4 timeout.

## Session

1. The client connects, subscribes to TX notifications and sends HELLO on RX
   (sequence 0). The server sends its HELLO on TX once both characteristics are
   in use. Both HELLOs may cross; neither side sends DATA before it has
   received the peer's HELLO.
2. A side that receives DATA or CREDIT before HELLO, a second HELLO, or an
   unsupported version closes the link with reason 2.
3. After the handshake the server starts the SSH session; the stream then
   carries raw SSH protocol bytes in both directions.
4. Either side ends the session with CLOSE and the server then drops the BLE
   connection. A BLE disconnect without CLOSE ends the session as well; the
   server never resumes a session on a new connection.

The server allows the handshake 20 s (default) after the client starts using
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
  `canSendWriteWithoutResponse`).
* Frame size = negotiated ATT MTU − 3 (Android: request MTU 517 first).
* Pass DATA bytes to the SSH layer unchanged; never decode text per packet.

## Test vectors

```
HELLO v1 window=0x4000, seq 0 : 01 00 00 42 4c 54 4d 01 00 00 40 00 00
DATA "ab", seq 0x0102         : 02 02 01 61 62
CREDIT 0x01020304, seq 7      : 03 07 00 04 03 02 01
CLOSE normal "ok", seq 9      : 04 09 00 00 6f 6b
```
