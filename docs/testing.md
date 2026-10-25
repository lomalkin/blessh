# Testing ble-term

Checks come in three tiers by cost. The hardware procedure (tier 3 and the
bench part of tier 2) records its results in [hardware.md](hardware.md):
`pass`, `fail` (with a link to the issue or a note), or `—` (not run) per
case.

## Tier 1: CI on every push

Cheap and short; GitHub runners, `.github/workflows/ci.yml`. A push is not
done until these pass.

| Check | How |
|---|---|
| Formatting, lints | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` |
| Protocol state machine (Rust) | `cargo test` |
| OpenSSH ↔ BleStream ↔ `sshd -i`, MTU 20/185/512, packet reordering, 300 KB each way, PTY; session socket with `ble-term-session` | `tests/e2e-loopback.sh` |
| Licences, sources, RustSec advisories (main and fuzz crate) | `cargo deny check` (`deny.toml`) |
| Fuzzing smoke test, 30 s per target | job `fuzz-smoke` (`cargo +nightly fuzz run --fuzz-dir tests/fuzz …`) |
| Packages for arm64, amd64, armhf; ARMv6 unit tests on an emulated ARM1176 | jobs `deb`, `deb-armhf` |
| Package in clean bookworm and trixie: BLE sessions get the system SSH configuration, purge | job `package` (`tests/e2e-package.sh`) |
| Protocol state machine (Kotlin, shared test vectors) | `BleStreamLinkTest` in the ConnectBot fork CI |

## Tier 2: long checks and bench tests, on request

Run on request, after substantial changes to the protocol, the daemon or
the packaging, and always before a release. They take tens of minutes to
hours and need root, a lot of memory or the bench, so they are not in CI.

| Check | How |
|---|---|
| Real BlueZ GATT path with two virtual controllers (root, `hci_vhci`) | `tests/e2e-vhci.sh`; again with `DAEMON_USER=nobody` (D-Bus policy) and with a small ATT MTU (`ExchangeMTU = 48`, `Channels = 1` in bluetoothd's configuration) |
| Undefined behaviour | `cargo +nightly miri test --lib --no-default-features` (about 40 min) |
| Model checking: no frame of up to 16 bytes makes the decoder panic | `cargo kani --no-default-features --lib` (about a minute) |
| AddressSanitizer, LeakSanitizer | `RUSTFLAGS=-Zsanitizer=address cargo +nightly test --target x86_64-unknown-linux-gnu`, and the loopback test with such a build |
| Fuzzing, up to an hour per target | `cargo +nightly fuzz run --fuzz-dir tests/fuzz <target>`; overnight runs only when asked for |
| Bench: client and server over the radio (`ble-term-connect` on one bench node, server on another), both directions | `tests/e2e-bench.sh <address> <name>` on the client node (see the script; about 3 minutes for 30 KB) |
| Bench: install and upgrade | S1, S13 and the other cases below that need no phone |

Bench nodes are shared: before a run, say which nodes it uses and for how
long, and nobody else touches them until the run is reported finished.
Host names and addresses of the bench stay out of the repository (see
[AGENTS.md](AGENTS.md)).

## Tier 3: phone, before a release

The cases below that need a phone (S2–S9 at least), run by a person on
each bench server, from the installation of the release candidate.

## Setup

Server:

```
sudo apt install ./ble-term_<version>_<arch>.deb
```

The output shows the SSH host key fingerprint (the system's, the same as
for network SSH: `256 SHA256:… root@<host> (ED25519)`), note it, and the
login policy BLE sessions get from `/etc/ssh/sshd_config` (passwords, root
login, allowed users). If Bluetooth is off, the
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

Android client (ConnectBot BLE, package `io.github.aluminique.connectbot.ble`,
installs next to a Play Store ConnectBot):

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
| S1 | Install | Package installs, fingerprint and login policy printed, both units active, journal shows `ready` |
| S2 | Discovery | The server appears in the client's scan |
| S3 | Login | First connect shows the installer's fingerprint; logins succeed and fail as over network SSH (key login; password login only where the system allows it) |
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
| S15 | D-Bus policy | `sudo -u _bleterm dbus-send --system --print-reply --dest=org.bluez /org/bluez/hci0 org.bluez.Adapter1.StartDiscovery` fails with `AccessDenied`; S2, S3, S7 and S8 pass |

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
