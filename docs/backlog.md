# Backlog

Ideas worth doing, not scheduled yet, and open points from the blessh
rework (2026-10).

## macOS

* The GATT stream on macOS (client and server): CoreBluetooth
  notifications and writes without response, gated on
  `canSendWriteWithoutResponse`; until then macOS needs L2CAP peers.
* Wake the channel thread on writes instead of its 10 ms tick (a run loop
  source), and bound sessions per device as on Linux.
* A signed `.pkg` instead of the install script; a Homebrew formula for
  the client.
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
* Selection by name: bluetoothd reports the GAP device name once it has
  read it, which can differ from the advertised `--name`; select by the
  advertised name only, or document the difference.
* Session resumption over a new link (would need a layer above raw SSH).
* GATT stream with several sessions on BlueZ ≥ 5.69 (per-device
  sockets), if a GATT-only client ever needs it.
* The vhci test with `DAEMON_USER` (D-Bus policy) in CI, which needs
  `hci_vhci` on the runner.

## Repository

* `ci.yml` still runs for pull requests from the rework branch; replace it
  with `blessh.yml` when the branch merges.
* Release notes and `docs/releases/` for the first blessh release.
