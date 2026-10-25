# ble-term

SSH sessions over Bluetooth Low Energy for Linux (BlueZ). Reference
platform: Raspberry Pi with Raspberry Pi OS / Debian Bookworm or Trixie
(64-bit `arm64` and 32-bit `armhf` packages). Tested devices:
[docs/hardware.md](docs/hardware.md).

```
phone (ConnectBot BLE)  ──BLE GATT──▶  bluetoothd ──D-Bus/fd──▶ ble-term-daemon (_bleterm)
                                                                     │ AF_UNIX
                                       systemd ble-term-session.socket (Accept=yes)
                                                                     ▼
                                       ble-term-session (logs the BLE device) → exec
                                       sshd -i -f /etc/ble-term/sshd_config  →  PAM, PTY, shell
```

* **No BlueZ reconfiguration.** The daemon registers a GATT service and an
  advertisement over D-Bus; no `main.conf` changes, no `--experimental`, no
  pairing agent. Other Bluetooth devices and services keep working. (On
  BlueZ before 5.67 EATT is kept off through a kernel setting, see
  "Bookworm" below.)
* **No pairing.** Security comes from SSH: the client pins the host key,
  users log in with their system password (PAM) or `~/.ssh/authorized_keys`.
* **One more way into the system SSH server.** BLE sessions get the
  configuration of `/etc/ssh/sshd_config` (authentication, allowed users,
  host keys): whatever is allowed or refused over the network is allowed or
  refused over BLE.
* **Unprivileged daemon.** Radio input is parsed by `_bleterm`; the only root
  code is `ble-term-session`, which reads one line from the daemon and execs
  OpenSSH's `sshd`, started per connection by systemd. The network SSH
  service can stay disabled.
* **Session lifetime = BLE connection.** On disconnect sshd gets EOF and the
  session receives SIGHUP, exactly like a dropped network SSH connection.
* One client at a time (v1).

Wire protocol: [docs/protocol.md](docs/protocol.md); what must stay
compatible with installed clients: [docs/compatibility.md](docs/compatibility.md).
Test procedure: [docs/testing.md](docs/testing.md). Repository conventions:
[docs/AGENTS.md](docs/AGENTS.md).

## Install

Packages are on the repository's Releases page: `arm64` for 64-bit and
`armhf` for 32-bit Raspberry Pi OS (every Pi, including Pi Zero and Pi 1),
`amd64` for PCs. Check the download against `SHA256SUMS`, then install:

```
sha256sum -c --ignore-missing SHA256SUMS
sudo apt install ./ble-term_<version>_arm64.deb
```

The package creates the `_bleterm` user (allowed only the BlueZ calls the
daemon needs, through a D-Bus policy), enables and starts
`ble-term-session.socket` and `ble-term.service`, and prints the SSH host
key fingerprint and the login policy BLE sessions get. Show the fingerprint
again with

```
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

### SSH settings

BLE sessions run `sshd -i -f /etc/ble-term/sshd_config`, and that file only
includes `/etc/ssh/sshd_config`: the system SSH server's settings and host
keys apply, with its `sshd_config.d` drop-ins. `ssh.service` itself may stay
disabled. Settings meant for BLE sessions only go above the `Include` line
in `/etc/ble-term/sshd_config` (sshd uses the first value it reads). sshd
sees no source address for BLE clients, so `Match Address` blocks do not
apply to them. Show what BLE sessions get:

```
sudo sshd -T -f /etc/ble-term/sshd_config
```

Before 0.2.0 the package had its own host key and its own settings
(passwords allowed); see [docs/releases/v0.2.0.md](docs/releases/v0.2.0.md).

Logs: `journalctl -u ble-term -u 'ble-term-session@*'`.

Audit: every session gets a random id. `ble-term.service` logs it with the
client's BLE address when the session starts, and with duration, byte counts
and how it ended when it is over. The session's `ble-term-session@…` unit
logs the same id and address, followed by sshd's login lines. Note that
phones use private BLE addresses that change every few minutes, so the
address identifies a connection rather than a phone.

Upgrades restart the daemon, which ends an active BLE session; the client
has to reconnect.

BLE logins are possible while `ble-term.service` and
`ble-term-session.socket` run. Stop until the next boot:
`sudo systemctl stop ble-term.service`. Disable:
`sudo systemctl disable --now ble-term.service ble-term-session.socket`.
Remove: `sudo apt purge ble-term`.

Requirements: BlueZ ≥ 5.55 (tested design targets 5.66 and 5.82), OpenSSH
server package (only the `sshd` binary is used), systemd. The Bluetooth adapter
must be on; the daemon waits for it and logs what to do if it is blocked by
rfkill (`sudo rfkill unblock bluetooth`) or powered off
(`bluetoothctl power on`). The package does not change the rfkill state.

## Security

Anyone within Bluetooth range can connect to the server and try to log in;
the BLE link is neither paired nor encrypted, and SSH provides
authentication and encryption. Recommended:

* Compare the host key fingerprint on the first connection.
* Use public keys and turn passwords off (`PasswordAuthentication no`,
  `KbdInteractiveAuthentication no` in the system SSH configuration; this
  applies to BLE too). If passwords stay on, use strong ones; sshd cannot
  rate-limit BLE clients by source.
* Limit who may log in over BLE only with `AllowGroups` above the
  `Include` line in `/etc/ble-term/sshd_config`.
* Use BlueZ 5.76 or later (Debian trixie). With older BlueZ, other BLE
  devices nearby can break an active session or observe its encrypted
  traffic and keystroke timing; the installer warns about this.

Threat model, findings and checks: [docs/SECURITY.md](docs/SECURITY.md).

## Bookworm (BlueZ 5.66): EATT and pairing requests

BlueZ before 5.67 enables EATT (several ATT channels per connection,
`Channels = 3`) by default. When a phone connects, bluetoothd opens an extra
EATT channel to it; the phone accepts EATT only on an encrypted link, so
bluetoothd asks the phone to pair. ble-term does not use pairing. Cancel or
no answer drops the connection (no answer: after 30 s); accepting fails
("pairing rejected") but the session survives, and the request returns once
the phone changes its random address. BlueZ 5.67 turned EATT off
by default (upstream commit "main: Disable EATT by default"), so Debian 13
and later are not affected.

ble-term handles this itself. On BlueZ before 5.67, `ble-term.service`
first runs `/usr/lib/ble-term/eatt-off`, which turns off the kernel's
enhanced credit based L2CAP mode (ECRED, the
`enable_ecred` parameter of the `bluetooth` module). EATT needs it, so
bluetoothd's EATT attempt fails quietly and the phone is never asked to
pair: the same behaviour as BlueZ 5.67 and later. Neither bluez's
configuration nor bluetoothd is touched; the journal of `ble-term.service`
says when this happens. Effects and limits:

* Lasts until reboot and is renewed on every start of the service;
  removing the package turns ECRED back on.
* Applies to the whole system. On these BlueZ versions only EATT and the
  experimental LE Audio support use ECRED.
* An explicit `Channels` setting in the `[GATT]` section of
  `/etc/bluetooth/main.conf` is left alone: `Channels = 1` already turns
  EATT off; a larger value means the administrator wants EATT, and phones
  will then be asked to pair.

## Desktop client

`ble-term-connect` (in the same package, or `cargo build --release`) is an
OpenSSH ProxyCommand for Linux machines with BlueZ:

```
ble-term-connect --scan                       # list servers
ssh -o ProxyCommand='ble-term-connect AA:BB:CC:DD:EE:FF' -o HostKeyAlias=my-pi user@my-pi
```

The client machine needs EATT off as well (`Channels = 1` in the `[GATT]`
section of `/etc/bluetooth/main.conf`, the default since BlueZ 5.67);
`ble-term-connect` warns if it is on.

## Development

```
cargo test                          # protocol unit tests
cargo build && tests/e2e-loopback.sh   # ssh ↔ simulated BLE ↔ sshd -i, no radio needed
```

Releases: set `version` in `Cargo.toml`, commit, then push a matching tag
(`git tag v0.1.2 && git push origin v0.1.2`). CI builds the packages and
publishes a release with `SHA256SUMS` and the commit subjects since the
previous tag. Untagged CI builds are versioned `X.Y.Z~ciN`, which sorts
before the `X.Y.Z` release.

`ble-term-loopback` runs both protocol ends over a simulated link (small MTU,
packet reordering) and is what the end-to-end test uses.
