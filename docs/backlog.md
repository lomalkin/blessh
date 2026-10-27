# Backlog

Ideas worth doing, not scheduled yet, and open points from the blessh
rework (2026-10).

## macOS

* The GATT stream on macOS (client and server): CoreBluetooth
  notifications and writes without response, gated on
  `canSendWriteWithoutResponse`; until then macOS needs L2CAP peers.
* Wake the channel thread on writes instead of its 10 ms tick (a run loop
  source), and bound sessions per device as on Linux.
* Packaging: a Homebrew tap with two separate formulae, `blessh-client`
  (the common case on macOS) and `blessh-server` (`brew services` for the
  LaunchAgent, a `sudo` step for the sshd LaunchDaemon); no meta formula,
  as the two have different requirements. Built locally until the
  sessions are reliable; later a signed `.pkg`.
* The server as an `.app` bundle: a stable Bluetooth permission (TCC
  keys it to the code signature; today it is the terminal app's, and an
  unsigned binary may be asked again after an update), its own entry in
  the Bluetooth and Login Items settings (SMAppService), and a Homebrew
  cask. Needs the bundle layout, SMAppService registration, and for other
  machines a Developer ID signature and notarization.
* Server names on macOS come from HostName or ComputerName at every
  start, without the " (N)" macOS appends on a network conflict; a name
  the user really ends in " (N)" loses it as well. A name like Linux's
  `/etc/hostname` that macOS never changes does not exist; watch for
  other renames (LocalHostName gets "-N", unused here).
* Throughput over the channel is low (about 10 KB/s per direction
  between a Mac and Linux, 10 to 30 KB/s between Linux machines): look at
  the channel thread's buffers and tick, the connection interval the
  central asks for, and the L2CAP credits.
* A user-level sshd cannot serve sessions on macOS (BSM audit needs
  root): tests against a macOS server need the LaunchDaemon.
* Verify on hardware: CoreBluetooth against BlueZ over the channel, the
  permission prompt for a LaunchAgent, sleep and logout behaviour.

## Clients

* Android: L2CAP channel in ConnectBot BLE (`createInsecureL2capChannel`,
  API 29); the GATT stream keeps working meanwhile.
* iOS app, sharing the CoreBluetooth code with the macOS client.
* Windows client over WinRT GATT (no L2CAP API): `blessh proxy` only.
* Web client (Web Bluetooth + WASM SSH): origin trust and supply chain
  still to be decided; `web/` is reserved for it.

## Server and protocol

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

* `ci.yml` still runs for pull requests from the rework branch; replace it
  with `blessh.yml` when the branch merges.
* Release notes and `docs/releases/` for the first blessh release.
