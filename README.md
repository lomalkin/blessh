# blessh

SSH sessions over Bluetooth Low Energy: `blessh alice@my-pi` from a laptop,
with no pairing, no network and no IP address to know. The server runs on
Linux with BlueZ; the client in this repository runs on Linux, the Android
client is ConnectBot BLE, and macOS, iOS, Windows and web clients follow the
same protocol. Principles, platforms and components:
[docs/architecture.md](docs/architecture.md).

```
blessh (laptop) ──BLE L2CAP channel, or GATT stream──▶ kernel / bluetoothd ──▶ blessh-server (_blessh)
                                                                                     │ AF_UNIX
                                                            systemd blessh-session.socket (Accept=yes)
                                                                                     ▼
                                                            blessh-session (logs the BLE device) → exec
                                                            sshd -i -f /etc/blessh/sshd_config → PAM, PTY, shell
```

* **No pairing.** The BLE link is neither paired nor encrypted; SSH does
  all of it: the client pins the host key, users log in with their system
  password (PAM) or `~/.ssh/authorized_keys`.
* **One more way into the system SSH server.** BLE sessions get the
  configuration of `/etc/ssh/sshd_config` (authentication, allowed users,
  host keys): whatever is allowed or refused over the network is allowed or
  refused over BLE.
* **No BlueZ reconfiguration.** The daemon registers a GATT service and an
  advertisement over D-Bus and listens on an L2CAP channel through the
  kernel; no `main.conf` changes, no `--experimental`, no pairing agent.
  Other Bluetooth devices and services keep working. (On BlueZ before 5.67
  EATT is kept off through a kernel setting, see "Bookworm" below.)
* **Unprivileged daemon.** Radio input is parsed by `_blessh`; the only root
  code is `blessh-session`, which reads one line from the daemon and execs
  OpenSSH's `sshd`, started per session by systemd. The network SSH service
  can stay disabled.
* **Several sessions.** Every L2CAP channel is one session: a client may
  open several, and several clients may connect, up to a limit (4 by
  default). The GATT stream carries one session at a time.
* **Session lifetime = BLE channel.** When the channel or the link goes
  away, sshd gets EOF and the session receives SIGHUP, exactly like a
  dropped network SSH connection.

Wire protocol: [docs/protocol.md](docs/protocol.md); what installed clients
rely on: [docs/compatibility.md](docs/compatibility.md). Test procedure:
[docs/testing.md](docs/testing.md). Repository conventions:
[docs/AGENTS.md](docs/AGENTS.md).

## Install the server

Packages are on the repository's Releases page: `arm64` for 64-bit
Raspberry Pi OS and Debian, `amd64` for PCs. Check the download against
`SHA256SUMS`, then install:

```
sha256sum -c --ignore-missing SHA256SUMS
sudo apt install ./blessh-server_<version>_arm64.deb
```

The package creates the `_blessh` user (allowed only the BlueZ calls the
daemon needs, through a D-Bus policy), enables and starts
`blessh-session.socket` and `blessh-server.service`, and prints a table of
checks (BlueZ version, Bluetooth service and adapter, sshd configuration,
daemon; each with OK, WARN or FAIL and what to do), the SSH host key
fingerprint and the login policy BLE sessions get. `sudo dpkg-reconfigure
blessh-server` prints it again. Show the fingerprint alone with

```
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

Settings go into `/etc/default/blessh` as `BLESSH_ARGS`: `--name` (the
advertised name, default: the host name), `--adapter`, `--max-sessions`
(default 4; `MaxConnections` in `blessh-session.socket` must allow as
many), `--no-l2cap` (serve the GATT stream only); `blessh-server --help`
lists them all. Restart the service after a change.

### SSH settings

BLE sessions run `sshd -i -f /etc/blessh/sshd_config`, and that file only
includes `/etc/ssh/sshd_config`: the system SSH server's settings and host
keys apply, with its `sshd_config.d` drop-ins. `ssh.service` itself may stay
disabled. Settings meant for BLE sessions only go above the `Include` line
in `/etc/blessh/sshd_config` (sshd uses the first value it reads). sshd
sees no source address for BLE clients, so `Match Address` blocks do not
apply to them. Show what BLE sessions get:

```
sudo sshd -T -f /etc/blessh/sshd_config
```

Logs: `journalctl -u blessh-server -u 'blessh-session@*'`.

Audit: every session gets a random id. `blessh-server.service` logs it with
the client's BLE address and the transport (`l2cap` or `gatt`) when the
session starts, and with duration, byte counts and how it ended when it is
over. The session's `blessh-session@…` unit logs the same id, address and
transport, followed by sshd's login lines. Note that phones use private BLE
addresses that change every few minutes, so the address identifies a
connection rather than a phone.

Upgrades restart the daemon, which ends the active BLE sessions.

BLE logins are possible while `blessh-server.service` and
`blessh-session.socket` run. Stop until the next boot:
`sudo systemctl stop blessh-server.service`. Disable:
`sudo systemctl disable --now blessh-server.service blessh-session.socket`.
Remove: `sudo apt purge blessh-server`.

Requirements: BlueZ ≥ 5.55 (tested with 5.66, 5.72 and 5.82), a kernel
with LE L2CAP connection-oriented channels (any current one; before Linux
6.10 the kernel grants receive credits without regard to the socket
buffer, which only matters when sshd stops reading), the OpenSSH server
package (only the `sshd` binary is used), systemd. The Bluetooth adapter
must be on; the daemon waits for it and logs what to do if it is blocked
by rfkill (`sudo rfkill unblock bluetooth`) or powered off
(`bluetoothctl power on`). The package does not change the rfkill state.

`blessh-server.service` starts with `bluetooth.target` and, for boots where
that target does not come up (a systemd soft-reboot, where the adapter is
already there), with `multi-user.target`; it then starts bluetoothd through
D-Bus activation if `bluetooth.service` is enabled.

### Desktop systems

Desktop Bluetooth settings (KDE, GNOME, blueman) switch the adapter off or
block it with rfkill, and restore that state at login; blessh then waits
until Bluetooth is on again. Some desktops record "off" by mistake when the
system shuts down, because bluetoothd stops before the desktop session; the
adapter is then off after every reboot. Fix it in the desktop settings
(KDE: Bluetooth, "When logging in": enable Bluetooth) or in the system
(`After=bluetooth.service` for `user@.service`, so sessions stop first).

## Connect from Linux

Install `blessh-client_<version>_<arch>.deb` (it needs BlueZ and the
OpenSSH client), then:

```
blessh scan                        # servers in range: address, name, RSSI
blessh alice@my-pi                 # by the advertised name
blessh alice@AA:BB:CC:DD:EE:FF     # by Bluetooth address
blessh -p 22 -L 8080:localhost:80 alice@my-pi   # any ssh option or command
```

`blessh` runs the system ssh with a ProxyCommand of `blessh proxy` and a
HostKeyAlias (`blessh-<name>` or `blessh-<address>`), so the server's host
key is pinned under that name in `~/.ssh/known_hosts`; compare the
fingerprint on the first connection. One line before the session says which
transport is in use: `blessh: my-pi AA:BB:CC:DD:EE:FF: l2cap, mtu 1024`, or
`gatt, frame 497` with a server that offers no channel. `BLESSH_PROXY_ARGS`
passes options to the proxy, for example `--adapter hci1` or `--gatt`;
`blessh proxy --help` lists them. The same works from `ssh_config`:

```
Host my-pi
    ProxyCommand blessh proxy my-pi
    HostKeyAlias blessh-my-pi
```

Several sessions to one server share its BLE link; the last `blessh` to
exit disconnects it.

The client machine needs no settings for the L2CAP channel. The GATT
stream (servers run with `--no-l2cap`, or `--gatt`) needs EATT off on the
client as well (`Channels = 1` in the `[GATT]` section of
`/etc/bluetooth/main.conf`, the default since BlueZ 5.67); `blessh` warns
if it is on.

If one of the two machines also publishes a GATT service that requires
encryption, the other machine's bluetoothd may try to pair with it on its
own; the connection then hangs or drops. Seen with PipeWire's WirePlumber,
which publishes BLE MIDI, against BlueZ's MIDI plugin on the other side,
whichever side is the server; phones are not affected. Turn off BLE MIDI in
WirePlumber, or start the other machine's bluetoothd without the MIDI
plugin (`-P midi`).

## macOS

The macOS build (`blessh-<version>-macos-arm64.tar.gz` on the Releases
page) has the same client and a server on CoreBluetooth; both use the
L2CAP channel (the GATT stream is not available on macOS yet, so the
server needs clients with L2CAP: Linux, macOS, Android 10 and later, iOS).
Unpack and run `sudo ./install.sh` from the account that should serve BLE
sessions: the BLE server runs as that user through a LaunchAgent, because
macOS grants Bluetooth to logged-in users only, and `sshd -i` runs as root
through a LaunchDaemon with `inetdCompatibility`, like Apple's own ssh job.
`/etc/blessh/sshd_config` includes the system `sshd_config`; Remote Login
does not have to be on. Sessions are logged in `/tmp/blessh-server.log`
and `/var/log/blessh-session.log`. `./uninstall.sh` removes it all.

The first use of Bluetooth from a terminal makes macOS ask for permission,
attributed to the terminal app; until it is granted, `blessh` and
`blessh-server` report "macOS has not decided whether this program may use
Bluetooth". macOS shows no Bluetooth addresses: `blessh scan` lists
CoreBluetooth's device identifiers instead, and `blessh user@<name>`
selects by advertised name. The server is only reachable while the user
is logged in.

## Security

Anyone within Bluetooth range can connect to the server and try to log in;
the BLE link is neither paired nor encrypted, and SSH provides
authentication and encryption. Recommended:

* Compare the host key fingerprint on the first connection.
* Log in with public keys only: turn passwords off
  (`PasswordAuthentication no`, `KbdInteractiveAuthentication no` in the
  system SSH configuration; this applies to BLE too). Anyone in radio range
  can try passwords, in up to `--max-sessions` sessions at once, and sshd
  cannot rate-limit BLE clients by source (docs/SECURITY.md, S-4).
* Limit who may log in over BLE only with `AllowGroups` above the
  `Include` line in `/etc/blessh/sshd_config`.
* Use BlueZ 5.76 or later (Debian trixie) where the GATT stream is in use.
  With older BlueZ, other BLE devices nearby can break an active GATT
  session or observe its encrypted traffic and keystroke timing; the
  installer warns about this. L2CAP channels are not affected.

Threat model, findings and checks: [docs/SECURITY.md](docs/SECURITY.md).

## Bookworm (BlueZ 5.66): EATT and pairing requests

BlueZ before 5.67 enables EATT (several ATT channels per connection,
`Channels = 3`) by default. When a phone connects, bluetoothd opens an extra
EATT channel to it; the phone accepts EATT only on an encrypted link, so
bluetoothd asks the phone to pair. blessh does not use pairing. Cancel or
no answer drops the connection (no answer: after 30 s); accepting fails
("pairing rejected") but the session survives, and the request returns once
the phone changes its random address. BlueZ 5.67 stopped opening EATT to
devices that connected on their own and turned EATT off by default, so
Debian 13 and later are not affected, whatever `Channels` says.

blessh handles this itself. On BlueZ before 5.67, `blessh-server.service`
first runs `/usr/lib/blessh/eatt-off`, which turns off the kernel's
enhanced credit based L2CAP mode (ECRED, the `enable_ecred` parameter of
the `bluetooth` module). EATT needs it, so bluetoothd's EATT attempt fails
quietly and the phone is never asked to pair. The L2CAP channel uses the LE
credit based mode, which ECRED does not affect. Neither bluez's
configuration nor bluetoothd is touched; the journal of
`blessh-server.service` says when this happens. Effects and limits:

* Lasts until reboot and is renewed on every start of the service;
  removing the package turns ECRED back on.
* Applies to the whole system. On these BlueZ versions only EATT and the
  experimental LE Audio support use ECRED.
* An explicit `Channels` setting in the `[GATT]` section of
  `/etc/bluetooth/main.conf` is left alone: `Channels = 1` already turns
  EATT off; a larger value means the administrator wants EATT, and phones
  will then be asked to pair.

## Development

```
cargo test                              # protocol, relay and client unit tests
cargo build && tests/e2e-loopback.sh    # ssh ↔ simulated BLE ↔ sshd -i, no radio needed
tests/e2e-vhci.sh                       # both transports over BlueZ with btvirt (root; docs/testing.md)
tests/docker/build-deb.sh               # the Debian packages, in a Debian container, into dist/
tests/docker/test-package.sh            # install, configuration checks and purge in a clean Debian
```

The workspace builds on macOS as well (the platform modules are stubs there
for now), so `cargo check` and the core tests work without BlueZ.

Releases: set `version` in the workspace `Cargo.toml`, commit, then push a
matching tag (`git tag blessh-v0.1.0 && git push origin blessh-v0.1.0`). CI
builds the packages and publishes a release with `SHA256SUMS` and the
commit subjects since the previous tag. Untagged CI builds are versioned
`X.Y.Z~ciN`, which sorts before the `X.Y.Z` release.

`blessh-loopback` runs both ends of the GATT stream over a simulated link
(small MTU, packet reordering) and is what the loopback test uses.

Without CI, `ci/local/build.sh blessh-vX.Y.Z` runs the same checks and
builds the same packages in docker on any Linux host, and writes the
release notes next to them; publish with `gh release create
blessh-vX.Y.Z <out>/*.deb <out>/SHA256SUMS --title "blessh X.Y.Z"
--notes-file <out>/notes.md`.
