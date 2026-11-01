Small fixes after the first release.

**Fixes**

* `blessh server disable` reports success once the server is stopped, and
  checks the Bluetooth permission as blessh rather than as the terminal.
* Commands that only print (`blessh scan`, `blessh server status`, help)
  end quietly when their output is closed early, for example by `head`.

**Android**

The Android client is [ConnectBot blessh](https://github.com/lomalkin/connectbot-blessh):
L2CAP channels from Android 10, the GATT stream before, servers by name or
address.
