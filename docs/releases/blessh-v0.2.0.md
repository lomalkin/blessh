The first release of blessh: SSH over Bluetooth Low Energy, with no
pairing and no network. One program, `blessh`, is the client and the
server, on Linux and macOS.

**Installing**

* macOS: `brew install lomalkin/tap/blessh`; to serve the Mac as well,
  `blessh server enable`.
* Debian, Ubuntu, Raspberry Pi OS: `blessh-client` (the program) and,
  for the server, `blessh-server`, from the files below; check them
  against `SHA256SUMS`.

**Upgrading from ble-term**

* `blessh-server` replaces the `ble-term` package. BLE sessions keep
  using the system's SSH configuration and host keys.
* The Android client (ConnectBot BLE) keeps working with Linux servers
  over the GATT stream. A macOS server offers only the L2CAP channel, so it
  needs clients with L2CAP: blessh on Linux and macOS, Android 10 and
  later.

Documentation: README, and for anyone writing a compatible client or
server, `docs/protocol.md`.
