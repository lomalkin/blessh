# What clients rely on

Installed clients (ConnectBot blessh on Android, blessh on Linux, later
macOS, iOS and web clients) must keep working with newer servers, and the
other way round, and so must other implementations. The specification is
[protocol.md](protocol.md); this page lists what a change must not break,
and how the protocol grows.

## Kept stable

* **Discovery:** the service UUID `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10`
  in the advertisement; the advertised name is the server's name.
* **GATT layout:** INFO `…01` (read), RX `…02` (write, write without
  response), TX `…03` (notify), PSM `abdd3056-…` (read). No
  characteristic starts requiring pairing or encryption.
* **INFO:** `key=value` pairs separated by `;`; `proto=1` and `name` are
  always present; clients ignore unknown keys.
* **L2CAP channels:** dynamic PSMs listed by INFO `psm` in the order to
  try them, the first also in the PSM characteristic; security level low;
  the channel handshake of INFO `chan` (magic `BLSH`, version, status,
  probe echo, options), then raw SSH; one session per channel.
* **GATT stream (BleStream v1):** header (type u8, sequence u16 LE), the
  four frame types and their payloads, sequence numbering of every frame,
  reorder window of 64, frame size min(ATT_MTU − 3, 500), one frame per
  ATT PDU with either write type on RX, the HELLO handshake (either side
  first, nothing before the peer's HELLO except HELLO or CLOSE with
  sequence 0), credit flow control, CLOSE reasons 0 to 4, `busy=1` in
  INFO while another device holds the stream. The test vectors in
  protocol.md stay valid and are checked by the Rust tests and by
  `BleStreamLinkTest` in the Android client.
* **Payload:** a plain SSH connection to the server's sshd with the
  system host keys; the package never changes them.
* **Behaviour:** the server ends a session when its channel or link goes
  away; a GATT client that subscribes while another device holds the
  stream gets CLOSE 1.

## How the protocol grows

* **INFO keys** may be added; `proto` and `name` stay. Added so far: `mtu`
  (ble-term 0.1.3), `busy` (0.2.3), `psm` and `chan` (blessh 0.1).
* **Channels** may be added to the `psm` list or reordered; clients try
  them in order and must accept any number.
* **Characteristics** may be added; existing UUIDs and properties stay.
* **Channel handshake:** options may be added (receivers ignore unknown
  types) and refusal reasons may change (clients show them, never parse
  them). A new handshake version is negotiated: the server answers with
  the lower version, and a server keeps answering every version it ever
  listed in `chan`. Flags stay 0 until a version defines them.
* **HELLO flags** and **trailing bytes** of the GATT stream: both sides
  send 0 and ignore what they do not know; a new optional feature gets a
  flag bit and is used only when both HELLOs set it.
* **New CLOSE reasons** may be added; receivers accept unknown reasons.
* **Not extensible without a version bump:** new GATT frame types and a
  different HELLO version (v1 receivers reject both); on a channel, the
  hello and reply headers of a handshake version.

## Not part of the contract

Server-internal details may change freely: the session socket and its
preamble, daemon options, unit names, log format, window sizes, MTUs,
timeouts, the session limit.
