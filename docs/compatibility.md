# What clients rely on

Installed clients (ConnectBot BLE on Android, blessh on Linux, later
macOS, iOS and web clients) must keep working with newer servers, and the
other way round. The normative description is [protocol.md](protocol.md);
this page lists what a change must not break, and how the protocol grows.

## Kept stable

* **Discovery:** the service UUID `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10`
  in the advertisement; the advertised name is the server's name.
* **GATT layout:** INFO `…01` (read), RX `…02` (write, write without
  response), TX `…03` (notify), PSM `abdd3056-…` (read). No
  characteristic starts requiring pairing or encryption.
* **INFO:** `key=value` pairs separated by `;`; `proto=1` and `name` are
  always present; clients ignore unknown keys.
* **L2CAP channel:** dynamic PSM named by INFO `psm` and the PSM
  characteristic; security level low; raw SSH from the first byte, except
  for refusal lines before the SSH banner; one session per channel.
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
  (ble-term 0.1.3), `busy` (0.2.3), `psm` (blessh 0.1.0).
* **Characteristics** may be added; existing UUIDs and properties stay.
* **Refusal lines** on the channel may gain reasons; clients show them at
  most and never parse them.
* **HELLO flags** and **trailing bytes** of the GATT stream: both sides
  send 0 and ignore what they do not know; a new optional feature gets a
  flag bit and is used only when both HELLOs set it.
* **New CLOSE reasons** may be added; receivers accept unknown reasons.
* **Not extensible without a version bump:** new GATT frame types and a
  different HELLO version (v1 receivers reject both), and the meaning of
  the first bytes on a channel (raw SSH or a refusal line).

## Not part of the contract

Server-internal details may change freely: the session socket and its
preamble, daemon options, unit names, log format, window sizes, MTUs,
timeouts, the session limit.
