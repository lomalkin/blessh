# Backlog

Ideas worth doing, not scheduled yet, and open points from the blessh
rework (2026-10).

## macOS

* The GATT stream on macOS (client and server): CoreBluetooth
  notifications and writes without response, gated on
  `canSendWriteWithoutResponse`; until then macOS needs L2CAP peers.
* Wake the channel thread on writes instead of its 10 ms tick (a run loop
  source), and bound sessions per device as on Linux.
* Logs of BLE sessions in sshd: today sshd logs them as coming from
  127.0.0.1 (macOS) or "UNKNOWN" (Linux), and the server's own log joins
  them by the source port (macOS) or the session unit (Linux). Possible
  later, if wanted despite touching the administrator's settings:
  `LogLevel VERBOSE` above the `Include` in `/etc/blessh/sshd_config`
  (sshd then logs the local port, which marks BLE sessions), and the
  server's lines in the unified log under a subsystem `org.blessh`. The
  criterion so far: as little change to the system and as few settings as
  possible.
* A signed `.pkg` for Macs without Homebrew.
* blessh as an `.app` bundle: macOS keys the Bluetooth permission of a
  program outside a bundle to its path (and its signature), which is why
  the formula runs blessh from a copy at a stable path
  (`var/lib/blessh/blessh`); a bundle would be keyed by its identifier
  instead, and would give its own entry in the Bluetooth and Login Items
  settings (SMAppService) and a Homebrew cask. Needs the bundle layout,
  SMAppService registration, and for other machines a Developer ID
  signature and notarization.
* Server names on macOS come from HostName or ComputerName at every
  start, without the " (N)" macOS appends on a network conflict; a name
  the user really ends in " (N)" loses it as well. A name like Linux's
  `/etc/hostname` that macOS never changes does not exist; watch for
  other renames (LocalHostName gets "-N", unused here).
* Throughput over the channel is low (about 10 KB/s per direction
  between a Mac and Linux, 10 to 30 KB/s between Linux machines): look at
  the channel thread's buffers and tick, the connection interval the
  central asks for, and the L2CAP credits.
* Verify on hardware: CoreBluetooth against BlueZ over the channel, the
  permission prompt for a LaunchAgent, sleep and logout behaviour.

## Links

* The Linux server drops a link thirty seconds after its last channel
  closed unless the client read INFO meanwhile. A client that reconnects
  within the seconds the disconnect takes to reach the radio (up to three
  on a Pi Zero 2 W) still loses its new session; with ten seconds this hit
  clients that came back right away, thirty moves it past the usual
  reconnect. Closing the gap needs a way to
  cancel or hold the disconnect, or the client retrying a channel cut
  right after the handshake.
* The Linux client that joins a link another blessh made takes the PSMs
  from the lock file and reads no INFO; it should read INFO too
  (protocol.md, "Lifetime").
* Large downloads from a macOS server to a Linux client on a controller
  without LE Data Length Extension (Pi Zero 2 W) stop at 64 KiB; uploads
  and interactive sessions work.

## Distribution

* An APT repository (for example on GitHub Pages), so that Debian and
  Raspberry Pi OS users get updates with `apt upgrade` instead of
  downloading packages from Releases.
* A short animation of a session in the README.

## Clients

* Android: L2CAP channel in ConnectBot BLE (`createInsecureL2capChannel`,
  API 29); the GATT stream keeps working meanwhile.
* iOS app, sharing the CoreBluetooth code with the macOS client.
* Windows client over WinRT GATT (no L2CAP API): `blessh proxy` only.
* Web client (Web Bluetooth + WASM SSH): origin trust and supply chain
  still to be decided; `web/` is reserved for it.

## Server and protocol

* Root cause of the channel stall between CoreBluetooth and Linux on
  controllers without LE Data Length Extension (Pi Zero 2 W, 27-byte LE
  buffer, MPS 23): CoreBluetooth sends 19-byte K-frames and stops in the
  middle of an SDU while Linux still counts credits for it; Linux returns
  credits only when its count reaches zero, because with MPS 23
  `l2cap_publish_rx_avail` always falls back to "one SDU" (the sk_buff
  overhead estimate exceeds the buffer). Seen in both directions; the
  channel handshake detects it and the second channel (server MTU 64) or
  a client MTU of 64 avoid it, at about 1 KB/s. Worth a kernel report with
  traces, and a look at whether CoreBluetooth or Linux breaks the spec.
* macOS client and a Pi Zero 2 W server: now and then CoreBluetooth
  reports the manager as powered off for a moment (often as another
  blessh process exits) and drops the peripheral, which ends the session
  (1 to 2 in 20 short sessions); not seen with other servers.
* A faster fallback than MTU 64 for such links (MTU 128 worked 2 of 3
  times); the right value may depend on the sender's K-frame size.

* Verify on real radios what btvirt cannot: two client devices at the
  same time (advertising while connected), phones, CoreBluetooth against
  BlueZ, and the Bookworm residual risk (a phone that answers bluetoothd's
  legacy fallback request on PSM 0x27 with "insufficient encryption" would
  still be asked to pair).
* PSM in the advertisement's service data, so Android clients can open
  the channel without a GATT round trip.
* Selection by name on Linux: bluetoothd may show the GAP device name
  instead of the advertised one; the client then asks servers whose name
  does not match for their INFO after a few seconds, which connects to
  every server in range. The name in the advertisement's service data
  would avoid that.
* `blessh scan` on Linux lists only devices whose advertised service
  UUID bluetoothd currently knows; it forgets them when a link drops,
  until it hears the device again.
* Session resumption over a new link (would need a layer above raw SSH).
* GATT stream with several sessions on BlueZ ≥ 5.69 (per-device
  sockets), if a GATT-only client ever needs it.
* The vhci test with `DAEMON_USER` (D-Bus policy) in CI, which needs
  `hci_vhci` on the runner.

## Repository

* Release notes and `docs/releases/` for the first blessh release.
