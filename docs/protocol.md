# blessh protocol

This is the specification of blessh on the wire: what a client and a
server exchange, and what each must do so that any client works with any
server. A client or server written from this document, in any language and
on any platform, is compatible with blessh; the implementation in this
repository follows it, and where they differ the document is right or the
difference is a bug. How the protocol may change without breaking
installed peers: [compatibility.md](compatibility.md).

A blessh server is a BLE peripheral that advertises one service. A client
connects, reads INFO and runs SSH over one of two transports: an L2CAP
connection-oriented channel (the main one) or the GATT stream BleStream v1
(the fallback). Either way the bytes are a plain SSH connection to the
server's SSH server.

The key words MUST, MUST NOT, SHOULD and MAY are used as in RFC 2119. The
whole document is normative, except platform notes, rationale and
sections whose title asks "why".

## Requirements

The rest of the document defines each item; this list is the checklist
for a new implementation.

A server:

* MUST advertise the service UUID and a name that follows the name rules
  ([Discovery](#discovery), [Names](#names)).
* MUST serve the GATT service with the INFO characteristic, readable
  without pairing or encryption, with long reads; INFO MUST contain
  `proto` and `name`, and the keys that describe the transports it offers
  ([GATT service](#gatt-service)).
* MUST offer at least one transport. A server with L2CAP channels MUST
  list them in `psm`, set `chan`, put the first PSM in the PSM
  characteristic, and run the channel handshake on every channel
  ([L2CAP channel](#l2cap-channel)). A server that does not serve the GATT
  stream MUST say so with `gatt=0`. A server SHOULD serve the GATT stream:
  clients without L2CAP (web, Windows, Android before 10) have nothing
  else.
* MUST carry one SSH session per channel, as plain bytes to an SSH server
  that uses the host's SSH configuration and host keys, and MUST end that
  session when the channel or the link goes away
  ([Lifetime](#lifetime)).
* MUST refuse a channel it cannot serve with a refusal reply, not by
  closing it silently.

A client:

* MUST select servers by the service UUID and by name or address, compare
  names as the name rules say, and ignore INFO keys it does not know.
* MUST use the L2CAP channels when the server lists them, `chan` names a
  handshake version the client speaks and the platform has L2CAP: try them
  in the listed order, run the handshake, and on a stall go on to the next
  one ([Channel handshake](#channel-handshake)). It MUST fall back to the
  GATT stream after the channels, unless INFO has `gatt=0`.
* MUST, on the GATT stream, follow the [client rules](#client-rules) and
  not subscribe while INFO has `busy=1`.
* MUST pass the SSH bytes unchanged and leave authentication to SSH; it
  SHOULD pin the server's host key per server name, since BLE names and
  addresses are not authenticated.
* MUST show refusal reasons and CLOSE messages to the user and MUST NOT
  act on their text.

Both sides MUST ignore unknown flags, options, keys and trailing bytes
where this document says so; that is how the protocol grows
([compatibility.md](compatibility.md)).

Checking an implementation: the [handshake](#channel-handshake) and
[BleStream](#test-vectors) test vectors below; against the implementation
in this repository, `blessh server run` and `blessh proxy` on Linux or
macOS, and `blessh-loopback` for the GATT stream without a radio
([testing.md](testing.md)).

## Discovery

The server advertises the service UUID `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10`
(16-bit list form is not used) and its name as the local name; the
advertising interval is 400 ms. Clients scan for the UUID and select a
server by name or by address. Neither is authenticated: the SSH host key is.

## Names

A server's name follows host name rules (RFC 1123 labels): ASCII letters,
digits and hyphens, parts separated by dots, no part empty or starting or
ending with a hyphen, at most 29 bytes (what fits the scan response of a
legacy advertisement). Names are
compared without regard to ASCII case, and a trailing dot is ignored. A
server that derives its name from the host's takes the first part of the
host name and replaces other characters with hyphens.

## GATT service

| Item | UUID | Properties |
|---|---|---|
| Service | `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10` | primary, advertised |
| INFO | `5b1e7e01-6d2c-4f4a-9a3e-2c1b0e8f4a10` | read: UTF-8 `key=value` pairs separated by `;` |
| RX (client → server) | `5b1e7e02-6d2c-4f4a-9a3e-2c1b0e8f4a10` | write, write without response: GATT stream |
| TX (server → client) | `5b1e7e03-6d2c-4f4a-9a3e-2c1b0e8f4a10` | notify: GATT stream |
| PSM | `abdd3056-28fa-441d-a470-55a75a52553a` | read: u16 little endian, present when the server offers a channel |

No characteristic requires encryption or pairing.

INFO keys; clients ignore unknown keys:

| Key | Value |
|---|---|
| `proto` | `1` |
| `name` | the advertised name |
| `mtu` | the ATT MTU of the bearer the read arrived on, for clients that cannot query it; read INFO after the MTU exchange |
| `psm` | decimal PSMs of the server's L2CAP channels, separated by `,`, in the order clients try them; absent when the server offers none |
| `chan` | version of the channel handshake the server speaks (`1`); present with `psm`. Clients that do not know the version use the GATT stream |
| `busy` | `1` while another device holds the GATT stream; absent otherwise. Concerns the GATT stream only |
| `gatt` | `0` when the server does not serve the GATT stream (a macOS server); absent otherwise |

The value can be longer than one ATT PDU; the server serves long reads
(Read Blob) from the requested offset.

The PSM characteristic carries the first PSM for iOS and macOS clients,
which look for Apple's `CBUUIDL2CAPPSMCharacteristicString`.

## L2CAP channel

The server listens on LE credit based connection-oriented channels with
dynamic PSMs (0x80 to 0xFF, assigned at start-up) and security level low:
the client connects without pairing or encryption. A Linux server offers
two channels that differ only in the server's receive MTU: 1024 bytes,
then 64 bytes (`--l2cap-mtu`, `--l2cap-small-mtu`). The second one exists
for pairs of Bluetooth stacks whose flow control stalls on large SDUs over
links without LE Data Length Extension (seen between macOS and a Linux
server whose controller has a 27-byte LE buffer); it is slow, so clients
use it only when the first one fails the handshake. A macOS server cannot
choose its MTU and offers one channel.

A channel is one session. It starts with the handshake below, then
carries the SSH connection in both directions, nothing else: the client's
SSH library talks to `sshd -i` as it would over TCP.

### Channel handshake

The client sends a hello right after the channel connects; the server
answers with a reply and echoes the probe. All integers are little endian.

```
hello  = "BLSH" version:u8 flags:u8 probe_len:u16 options_len:u16 options probe
reply  = "BLSH" version:u8 status:u8 probe_len:u16 options_len:u16 options echo
option = type:u8 len:u8 value
```

- `version`: the client sends the highest version it speaks (1); the
  server answers with the lower of that and its own, and both use it. A
  client aborts if the reply's version is higher than its own.
- `flags`: none defined; senders set 0, receivers ignore them.
- `status`: 0 accepted, 1 refused. A refused reply has `probe_len` 0 and
  a reason option; the server then closes the channel. Reasons are text
  for people, for example `busy (N sessions)` when the server's session
  limit is reached, or `session unavailable` when it cannot start the SSH
  server; clients show them and never parse them.
- `options`: at most 1024 bytes. Defined: type 1, the reason (UTF-8),
  in replies. Receivers ignore types they do not know, so later versions
  can add options without breaking older peers.
- `probe`: `probe_len` bytes (at most 8192) of any content; clients send
  2048, about the size of an SSH key exchange init, the first large message
  of a session. The server sends its reply as soon as it has read the
  hello's header and options, then writes back every probe byte as it
  arrives, before anything of the SSH session.

The client checks that the echo matches. If the reply or the echo stops
coming for 3 seconds (`--stall-timeout`), the channel counts as stalled:
the client closes it and tries the server's next channel. A client that
can choose its receive MTU (Linux) then tries each channel again with a
receive MTU of 64, for the same stall in the other direction. Last comes
the GATT stream, where the platform has one and the server serves it. A refusal ends the attempt: the limit
applies to every channel. The server waits 10 seconds for a hello and
closes channels that send something else.

The handshake shows that the channel carries data both ways with the
sizes SSH uses before SSH starts; without it, a stalled channel looks like
a hung login. Its cost is one round trip and the probe, about 0.1 to 0.3
seconds on a typical link.

Test vectors (version 1, a 4-byte probe `abcd`; real clients send 2048
bytes):

```
hello, probe "abcd"         : 42 4c 53 48 01 00 04 00 00 00 61 62 63 64
reply, accepted             : 42 4c 53 48 01 00 04 00 00 00   (then the echo 61 62 63 64)
reply, refused "busy"       : 42 4c 53 48 01 01 00 00 06 00 01 04 62 75 73 79
```

### Lifetime

The session ends when either side closes the channel or the link goes
down; the server then ends sshd's session (EOF, SIGHUP). A client may open
several channels on one link, one session each. The server does not
disconnect a link that carries a channel; ten seconds after the last
channel on a link closed, it may disconnect the link unless a new channel
opened or the client read INFO meanwhile. A client that reuses a link
SHOULD read INFO before it opens its channel, so that the link is not
taken down under it. Linux reports a peer's close or a lost link on an L2CAP
socket as an error (`ECONNRESET`, `ECONNABORTED`, `ETIMEDOUT`), not as
end of file; the errno names the HCI reason.

Platform notes: Linux clients open the channel through the kernel (`AF_BLUETOOTH`,
`BTPROTO_L2CAP`, `BT_SECURITY_LOW`), bluetoothd is not involved; Android
uses `BluetoothDevice.createInsecureL2capChannel` (API 29); iOS and macOS
`CBPeripheral.openL2CAPChannel`. Web Bluetooth and Windows have no L2CAP
API and use the GATT stream.

## GATT stream: BleStream v1

BleStream carries one reliable, ordered, flow-controlled byte stream in each
direction over the RX and TX characteristics. Every ATT write to RX and
every notification on TX carries exactly one frame. A frame never exceeds
min(ATT_MTU − 3, 500) bytes. The 500 byte cap leaves room below the 512
byte attribute limit: BlueZ acting as GATT client was seen to drop larger
writes without response when EATT is enabled.

The client writes RX with write without response or with write request
(write with response); the server treats both alike. Either way a frame
fits in one ATT PDU: long writes (Prepare/Execute Write), which split a
value across PDUs, are not allowed.

BleStream relies on the link layer for reliable, ordered delivery and
assumes a single ATT bearer: the connection's unenhanced ATT bearer. With
several bearers (EATT), frames can overtake each other by any number of
positions, so v1 does not support EATT; both sides should have it off
(BlueZ: `Channels = 1` in the `[GATT]` section of `main.conf`, the default
since 5.67; on older BlueZ the server package keeps EATT off by turning off
the kernel's ECRED mode; phones open EATT only on encrypted links, and
blessh links are not encrypted).

### Frames

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

CLOSE reasons: 0 normal, 1 busy (another client holds the stream, or the
session limit is reached), 2 protocol error, 3 session unavailable (server
could not start sshd, or is shutting down), 4 timeout.

### Session

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

### Why a turned-away client must not subscribe

The GATT stream carries one session at a time. INFO says `busy=1` while
another device holds it, and a client that reads it leaves without
subscribing: BlueZ closes another device's notification socket when a
subscribed device disconnects (5.82 to at least 5.87), and before 5.69 all
devices share one write socket, so a second subscriber can end the first
client's session. Clients that subscribe anyway get CLOSE 1, with that
risk. The L2CAP channel has none of this: every channel is its own socket.

### Flow control

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

### Client rules

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

### Test vectors

```
HELLO v1 window=0x4000, seq 0 : 01 00 00 42 4c 54 4d 01 00 00 40 00 00
DATA "ab", seq 0x0102         : 02 02 01 61 62
CREDIT 0x01020304, seq 7      : 03 07 00 04 03 02 01
CLOSE normal "ok", seq 9      : 04 09 00 00 6f 6b
```
