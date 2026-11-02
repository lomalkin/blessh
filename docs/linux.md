# blessh on Linux

The server and the client on Debian, Ubuntu and Raspberry Pi OS with
BlueZ. Installation in short: [README](../README.md#installing). The
macOS server: [macos-server.md](macos-server.md); the `blessh` command on
every platform: [client.md](client.md).

## Package

One package, `blessh`, on the Releases page: `arm64` for 64-bit Raspberry
Pi OS and Debian on ARM, `amd64` for PCs. It is the client and the
server, like the Homebrew formula on macOS; the server is off until it is
turned on. The installer script (README, "Installing") does the same as:

```
sha256sum -c --ignore-missing SHA256SUMS
sudo apt install ./blessh_<version>_arm64.deb
blessh server enable
```

The package installs the program, the systemd units (off), the `_blessh`
user (allowed only the BlueZ calls the daemon needs, through a D-Bus
policy; created on every machine, like other packages' system users),
`/etc/blessh/sshd_config` and `/etc/default/blessh`. A first installation
prints how to connect and how to turn the server on.

The server needs OpenSSH's server, which the package only suggests, so
that a machine used as a client does not get one: `sudo apt install
openssh-server` (it turns on network SSH too; BLE does not need it:
`sudo systemctl disable --now ssh`). `blessh server enable` refuses
without it and says so.

`blessh server enable` (it asks for sudo) enables and starts
`blessh-session.socket` and `blessh-server.service`, then prints what
`blessh server status` prints: a table of checks (BlueZ version, EATT on
Bookworm, Bluetooth service and adapters, sshd configuration, the server,
whether it shows in device lists; each OK, WARN, FAIL or NOTE with what
to do), the name, the SSH host key fingerprint and the login policy BLE
sessions get. `blessh server status` checks sshd's view of its
configuration only as root (`sudo blessh server status`). The fingerprint
alone:

```
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
```

Upgrades restart a server that is on, which ends the active BLE
sessions, and print its status.

## Server settings

Settings go into `/etc/default/blessh` as `BLESSH_ARGS`, then `sudo
systemctl restart blessh-server`:

| Option | Meaning |
|---|---|
| `--name NAME` | advertised name; default: the static host name from `/etc/hostname`, which neither DHCP nor avahi's renames on mDNS conflicts change |
| `--adapter hci1` | Bluetooth adapter; default: BlueZ's default adapter |
| `--max-sessions N` | sessions at a time over all transports (default 4; `MaxConnections` in `blessh-session.socket` must allow as many) |
| `--no-l2cap` | serve the GATT stream only |
| `--discoverable` | also show the server in the Bluetooth device lists of phones and computers (see below) |

`blessh server run --help` lists them all; `blessh server status` shows
the result.

## Visibility

The server advertises without the discoverable flags, so the Bluetooth
settings of phones and computers do not list it: tapping it there would
only start a pairing, which blessh does not use. blessh clients (`blessh`,
ConnectBot blessh) scan for the blessh service and find it all the same
([protocol.md](protocol.md#discovery)). This hides nothing: the
advertisement and its name are public, and any BLE scanner app shows
them. `--discoverable` puts the flags back.

## SSH settings

BLE sessions run `sshd -i -f /etc/blessh/sshd_config`, and that file only
includes `/etc/ssh/sshd_config`: the system SSH server's settings and host
keys apply, with its `sshd_config.d` drop-ins. `ssh.service` itself may
stay disabled. Settings meant for BLE sessions only go above the `Include`
line in `/etc/blessh/sshd_config` (sshd uses the first value it reads); the
file sets `SetEnv BLESSH=1` there, so a session can tell that it came over
BLE. sshd sees no source address for BLE clients, so `Match Address`
blocks do not apply to them. Show what BLE sessions get:

```
sudo sshd -T -f /etc/blessh/sshd_config
```

## Logs and audit

```
journalctl -u blessh-server -u 'blessh-session@*'
```

Every session gets a random id. `blessh-server.service` logs it with the
client's BLE address and the transport (`l2cap` or `gatt`) when the
session starts, and with duration, byte counts and how it ended when it is
over. The session's `blessh-session@…` unit logs the same id, address and
transport, followed by sshd's login lines. Phones use private BLE
addresses that change every few minutes, so the address identifies a
connection rather than a phone.

## Turning the server off

BLE logins are possible while `blessh-server.service` and
`blessh-session.socket` run.

```
sudo systemctl stop blessh-server   # until the next boot
blessh server disable               # also at boot (asks for sudo)
blessh server enable                # on again
sudo apt purge blessh               # remove the client and the server
```

Upgrades restart a server that is on, which ends the active BLE sessions.

## Requirements

* BlueZ 5.55 or later (tested with 5.66, 5.72, 5.82 and 5.85), systemd;
  for the server, the OpenSSH server package (only the `sshd` binary is
  used).
* A 64-bit system (`arm64`, `amd64`); 32-bit Raspberry Pi OS is not
  supported yet.
* A kernel with LE L2CAP connection-oriented channels: any current one.
  Before Linux 6.10 the kernel grants receive credits without regard to
  the socket buffer, which only matters when sshd stops reading.
* The Bluetooth adapter must be on. The daemon waits for it and logs what
  to do if it is blocked by rfkill (`sudo rfkill unblock bluetooth`) or
  powered off (`bluetoothctl power on`). The package does not change the
  rfkill state.

`blessh-server.service` starts with `bluetooth.target` and, for boots where
that target does not come up (a systemd soft-reboot, where the adapter is
already there), with `multi-user.target`; it then starts bluetoothd through
D-Bus activation if `bluetooth.service` is enabled.

## Desktop systems

Desktop Bluetooth settings (KDE, GNOME, blueman) switch the adapter off or
block it with rfkill, and restore that state at login; blessh then waits
until Bluetooth is on again. Some desktops record "off" by mistake when the
system shuts down, because bluetoothd stops before the desktop session; the
adapter is then off after every reboot. Fix it in the desktop settings
(KDE: Bluetooth, "When logging in": enable Bluetooth) or in the system
(`After=bluetooth.service` for `user@.service`, so sessions stop first).

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
