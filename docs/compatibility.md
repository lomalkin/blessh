# v1 compatibility requirements

Installed ConnectBot BLE builds (and later iOS or desktop clients) must keep
working with any newer ble-term server, and the other way round. Until a v2
is deliberately introduced, the following is frozen. The normative
description is [protocol.md](protocol.md).

## Frozen

* **GATT layout:** the service UUID `5b1e7e00-6d2c-4f4a-9a3e-2c1b0e8f4a10`
  (also in the advertisement, clients scan for it), INFO `…01` (read), RX
  `…02` (write and write without response), TX `…03` (notify). UUIDs and
  properties do not change; no characteristic starts requiring pairing or
  encryption.
* **Frames:** header (type u8, sequence u16 LE), the four types HELLO,
  DATA, CREDIT, CLOSE and their payloads, sequence numbering of every frame,
  reorder window of 64 (a tolerance; v1 assumes a single ATT bearer), frame
  size min(ATT_MTU − 3, 500), one frame per ATT PDU with either write type
  on RX. The test vectors
  in protocol.md stay valid and are checked by both the Rust tests and
  `BleStreamLinkTest` in the client.
* **Handshake:** HELLO with magic `BLTM` and version 1. Either side may send
  HELLO first; neither sends DATA before the peer's HELLO. Before the
  peer's HELLO, receivers ignore everything but a HELLO or CLOSE with
  sequence 0 (servers from 0.1.3; earlier receivers closed the link on
  such leftovers, which no sender relies on).
* **Flow control:** window from HELLO, CREDIT semantics, no DATA without
  credit, control frames not counted.
* **CLOSE reasons 0–4** keep their meaning.
* **Payload:** the stream carries a plain SSH connection to the server's
  sshd. Sessions use the system SSH host keys (since 0.2.0; before, the
  package had its own, and the 0.2.0 upgrade changed the key once); the
  package never changes them, otherwise every client sees a host key
  change.
* **Behaviour clients rely on:** the server ends the session on BLE
  disconnect; a second client gets CLOSE 1 (busy), and also sees `busy=1` in
  INFO before it subscribes.

## How v1 may be extended

* **HELLO flags.** Both sides send 0 and ignore bits they do not know. A new
  optional feature gets a flag bit and is used only when both HELLOs set it.
* **HELLO trailing bytes.** Receivers accept and ignore bytes after the
  10-byte HELLO payload; new HELLO fields go there, guarded by a flag.
* **INFO.** `key=value` pairs separated by `;`. Clients ignore unknown keys;
  new keys may be added, `proto` and `name` stay. Added so far: `mtu`
  (0.1.3), `busy` (0.2.3).
* **New CLOSE reasons** may be added. Receivers accept unknown reasons (the
  Rust code maps them to "protocol error", the Android client shows the
  number).
* **Not extensible:** new frame types (v1 receivers close the link on an
  unknown type) and a different HELLO version (v1 receivers reject it).
  Both require v2: a v2 server must still accept v1 clients, and since HELLOs
  may cross, it cannot wait for the client's HELLO before choosing its own.
  Plan v2 so that the client announces it first (for example through a new
  characteristic or an INFO key) before relying on it.

## Not part of the contract

Server-internal details may change freely: the session socket and its
preamble, daemon options, unit names, log format, window sizes (each side
announces its own), timeouts.
