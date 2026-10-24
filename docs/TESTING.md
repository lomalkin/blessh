# Testing ble-term on real hardware

This is the procedure for checking a server device (and a client) for
[HARDWARE.md](HARDWARE.md). Run the cases that apply, then add a row there
with the result of each case: `pass`, `fail` (with a link to the issue or a
note), or `—` (not run).

## Checks that need no hardware

Run on every change (CI does the same):

| Check | How |
|---|---|
| Protocol state machine (Rust) | `cargo test` |
| Protocol state machine (Kotlin, shared test vectors) | `BleStreamLinkTest` in the ConnectBot fork CI |
| OpenSSH ↔ BleStream ↔ `sshd -i`, MTU 20/185/512, packet reordering, 300 KB each way, PTY; session socket with `ble-term-session` | `tests/e2e-loopback.sh` |
| ARMv6 build: unit tests on an emulated ARM1176 | CI job `deb-armhf` |
| Real BlueZ GATT path with two virtual controllers (needs root and the `hci_vhci` module) | `tests/e2e-vhci.sh` |

## Setup

Server:

```
sudo apt install ./ble-term_<version>_<arch>.deb
```

The output ends with the BLE SSH host key fingerprint
(`256 SHA256:… ble-term@<host> (ED25519)`); note it. If Bluetooth is off, the
installer says so and how to switch it on. Then check:

```
systemctl status ble-term.service ble-term-session.socket
journalctl -u ble-term -n 20     # ends with "ready (BlueZ GATT application and advertisement registered)"
```

On Bookworm (BlueZ 5.66) apply the `Channels = 1` setting the installer
asks for (see README, "Bookworm"); without it the phone shows a pairing
request and the connection drops.

The daemon advertises the host name. To change it, set
`BLE_TERM_ARGS="--name my-pi"` in `/etc/default/ble-term` and run
`sudo systemctl restart ble-term`.

Android client (ConnectBot BLE build, package `org.connectbot.debug`, installs
next to a Play Store ConnectBot; uninstall an earlier test build if Android
refuses the update):

1. Hosts → **+** → protocol **ble** → **Scan for Bluetooth LE servers**,
   allow "Nearby devices", pick the server, enter the user name, save.
2. Tap the host, compare the fingerprint with the one from the installer,
   log in.

Desktop Linux client (optional):

```
sudo apt install ./ble-term_<version>_<arch>.deb
sudo systemctl disable --now ble-term.service ble-term-session.socket   # client-only machine
ble-term-connect --scan
ssh -o ProxyCommand='ble-term-connect <server-address>' -o HostKeyAlias=<name>-ble <user>@<name>-ble
```

## Cases

| ID | Case | Pass criteria |
|---|---|---|
| S1 | Install | Package installs, fingerprint printed, both units active, journal shows `ready` |
| S2 | Discovery | The server appears in the client's scan |
| S3 | Login | First connect shows the installer's fingerprint; password login (and key login, if used) succeeds |
| S4 | Full-screen programs | `htop`, `vim` or `nano`, `tmux` draw and respond correctly |
| S5 | Window resize | Rotating the phone (or resizing the terminal) resizes the remote terminal (`stty size`) |
| S6 | Bulk output | `cat /usr/share/common-licenses/GPL-3` or `dmesg` finishes; Ctrl+C during long output stops it |
| S7 | Exit and reconnect | `exit` ends the session; connecting again works without restarting anything |
| S8 | Link loss | Bluetooth off on the client, or out of range, mid-session: the session ends on the server (`who`; the journal shows `session …: ended`) and a later connect works |
| S9 | Second client | A second client while one is connected is refused with "another client is connected"; the first session is unaffected |
| S10 | Server reboot | After a reboot the units start, the server is discoverable, login works |
| S11 | Bluetooth off and on on the server | `bluetoothctl power off`, then `on`: the server becomes discoverable again and login works |
| S12 | Audit log | `journalctl -u ble-term` has session start and end lines with the device address and session id; the matching `ble-term-session@*` unit has the same id followed by the sshd login line |
| S13 | Upgrade | Installing a newer package over the old one ends the active session (expected) and the server works afterwards |
| S14 | Long session | A session idle or in use for an hour or more stays alive |

### Audit log (S12)

```
journalctl -u ble-term | grep 'session '
journalctl -u 'ble-term-session@*' --grep '<session id>'   # names the unit
journalctl -u 'ble-term-session@<n>-…'                      # that session's sshd lines
```

## What to report on failure

* `journalctl -u ble-term -u 'ble-term-session@*' --since '10 min ago'`
* `bluetoothctl --version`, `uname -r`, `/etc/os-release`
* Client: Android version and phone model; for app problems
  `adb logcat -d | grep -i -e BleStream -e "BLE " -e ConnectBot`

## Troubleshooting knobs

* `--readvertise` in `BLE_TERM_ARGS`: if the server stops being discoverable
  after the first session, this re-registers the advertisement after each
  session. Off by default because it broke reconnects in the
  virtual-controller tests.
* `RUST_LOG=debug` in `/etc/default/ble-term` (as an extra line) for verbose
  daemon logs.
