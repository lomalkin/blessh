# Testing blessh

Checks come in three tiers by cost. The hardware procedure (tier 3 and the
bench part of tier 2) records its results in [hardware.md](hardware.md):
`pass`, `fail` (with a link to the issue or a note), or `—` (not run) per
case.

## Tier 1: CI on every push

Cheap and short; GitHub runners, `.github/workflows/blessh.yml`. A push is
not done until these pass.

| Check | How |
|---|---|
| Formatting, lints | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` |
| Protocol state machine, relay, preamble, client argument parsing | `cargo test` |
| OpenSSH ↔ BleStream ↔ `sshd -i`, MTU 20/185/512, packet reordering, 300 KB each way, PTY; session socket with `blessh-session` | `tests/e2e-loopback.sh` |
| Licences, sources, RustSec advisories (main and fuzz crate) | `cargo deny check` (`deny.toml`) |
| Fuzzing smoke test, 30 s per target | job `fuzz-smoke` (`cargo +nightly fuzz run --fuzz-dir tests/fuzz …`) |
| Packages for arm64 and amd64 (both `blessh-server` and `blessh-client`) | job `deb`; locally `tests/docker/build-deb.sh` |
| Packages in clean bookworm and trixie: BLE sessions get the system SSH configuration, the client runs, purge | job `package` (`tests/e2e-package.sh`; locally `tests/docker/test-package.sh`) |
| macOS build, unit tests and the tarball with the launchd jobs | job `macos` |
| Protocol state machine (Kotlin, shared test vectors) | `BleStreamLinkTest` in the ConnectBot fork CI |

## Tier 2: long checks and bench tests, on request

Run on request, after substantial changes to the protocol, the daemon or
the packaging, and always before a release. They take tens of minutes to
hours and need root, a lot of memory or the bench, so they are not in CI.

| Check | How |
|---|---|
| Both transports over real BlueZ with two virtual controllers: L2CAP sessions, the ssh wrapper, two sessions on one link, the session limit, link loss, reconnects, the GATT stream with and without write responses (root, `hci_vhci`, `btvirt -L -l2`) | `tests/e2e-vhci.sh` (`SERVER_HCI`/`CLIENT_HCI` when a real adapter holds hci0); again with `DAEMON_USER=nobody` (D-Bus policy) and with a small ATT MTU (`ExchangeMTU = 48`, `Channels = 1` in bluetoothd's configuration) |
| Undefined behaviour | `cargo +nightly miri test -p blessh-core` (about 40 min) |
| Model checking: no frame of up to 16 bytes makes the decoder panic | `cargo kani -p blessh-core` (about a minute) |
| AddressSanitizer, LeakSanitizer | `RUSTFLAGS=-Zsanitizer=address cargo +nightly test --target x86_64-unknown-linux-gnu`, and the loopback test with such a build |
| Fuzzing, up to an hour per target | `cargo +nightly fuzz run --fuzz-dir tests/fuzz <target>`; overnight runs only when asked for |
| Bench: connecting and disconnecting, where most failures happen: many short sessions back to back and in overlapping pairs, logs of failed sessions kept | `tests/e2e-cycles.sh -n 10 -m all <name>` on the client node (Linux or macOS; `-t` bounds one session, 40 s by default) |
| Bench: client and server over the radio (installed packages on two bench nodes), both directions | `tests/e2e-bench.sh -s 1 <name>` (smoke, under a minute), `-s 2` (basics, a few minutes), `-s 3` (1 MB each way, 20 sessions in a row, link loss; about 10 minutes) on the client node; run the stages in this order |
| Bench: two client devices at the same time, phones, the macOS client and server against Linux and each other | by hand; not covered by btvirt (see architecture.md, "Verified so far"); the macOS binaries were built and started but could not reach Bluetooth without the permission prompt being answered |
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
sudo apt install ./blessh-server_<version>_<arch>.deb
```

The output shows the SSH host key fingerprint (the system's, the same as
for network SSH: `256 SHA256:… root@<host> (ED25519)`), note it, and the
login policy BLE sessions get from `/etc/ssh/sshd_config` (passwords, root
login, allowed users). If Bluetooth is off, the
installer says so and how to switch it on. Then check:

```
systemctl status blessh-server.service blessh-session.socket
journalctl -u blessh-server -n 20     # ends with "ready (BlueZ GATT application and advertisement registered, L2CAP channel on PSM 0x..)"
```

On Bookworm (BlueZ 5.66) the journal of `blessh-server.service` says that ECRED
was turned off (README, "Bookworm"); the phone must not show a pairing
request.

The daemon advertises the host name. To change it, set
`BLESSH_ARGS="--name my-pi"` in `/etc/default/blessh` and run
`sudo systemctl restart blessh-server`.

Android client (ConnectBot BLE, package `io.github.aluminique.connectbot.ble`,
installs next to a Play Store ConnectBot):

1. Hosts → **+** → protocol **ble** → **Scan for Bluetooth LE servers**,
   allow "Nearby devices", pick the server, enter the user name, save.
2. Tap the host, compare the fingerprint with the one from the installer,
   log in.

Desktop Linux client (optional):

```
sudo apt install ./blessh-server_<version>_<arch>.deb
sudo systemctl disable --now blessh-server.service blessh-session.socket   # client-only machine
blessh --scan
ssh -o ProxyCommand='blessh <server-address>' -o HostKeyAlias=<name>-ble <user>@<name>-ble
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
| S10 | Server reboot | After a reboot (and a `systemctl soft-reboot`, where the system uses it) the units start, the server is discoverable, login works |
| S11 | Bluetooth off and on on the server | `bluetoothctl power off`, then `on`: the server becomes discoverable again and login works |
| S12 | Audit log | `journalctl -u blessh-server` has session start and end lines with the device address and session id; the matching `blessh-session@*` unit has the same id followed by the sshd login line |
| S13 | Upgrade | Installing a newer package over the old one ends the active session (expected) and the server works afterwards |
| S14 | Long session | A session idle or in use for an hour or more stays alive |
| S15 | D-Bus policy | `sudo -u _blessh dbus-send --system --print-reply --dest=org.bluez /org/bluez/hci0 org.bluez.Adapter1.StartDiscovery` fails with `AccessDenied`; S2, S3, S7 and S8 pass |

### Audit log (S12)

```
journalctl -u blessh-server | grep 'session '
journalctl -u 'blessh-session@*' --grep '<session id>'   # names the unit
journalctl -u 'blessh-session@<n>-…'                      # that session's sshd lines
```

## What to report on failure

* `journalctl -u blessh-server -u 'blessh-session@*' --since '10 min ago'`
* `bluetoothctl --version`, `uname -r`, `/etc/os-release`
* Client: Android version and phone model; for app problems
  `adb logcat -d | grep -i -e BleStream -e "BLE " -e ConnectBot`

## Troubleshooting knobs

* `--readvertise` in `BLESSH_ARGS`: if the server stops being discoverable
  after the first session, this re-registers the advertisement after each
  session. Off by default because it broke reconnects in the
  virtual-controller tests.
* `RUST_LOG=debug` in `/etc/default/blessh` (as an extra line) for verbose
  daemon logs.
