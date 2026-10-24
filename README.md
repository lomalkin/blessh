# ble-term

SSH sessions over Bluetooth Low Energy for Linux (BlueZ). Reference
platform: Raspberry Pi 5 with Raspberry Pi OS / Debian Bookworm or Trixie.

```
phone (ConnectBot BLE)  ──BLE GATT──▶  bluetoothd ──D-Bus/fd──▶ ble-term-daemon (_bleterm)
                                                                     │ AF_UNIX
                                       systemd ble-term-session.socket (Accept=yes)
                                                                     ▼
                                       sshd -i -f /etc/ble-term/sshd_config  →  PAM, PTY, shell
```

* **No BlueZ reconfiguration.** The daemon registers a GATT service and an
  advertisement over D-Bus; no `main.conf` changes, no `--experimental`, no
  pairing agent. Other Bluetooth devices and services keep working.
* **No pairing.** Security comes from SSH: the client pins the BLE host key
  (`/etc/ble-term/ssh_host_ed25519_key`), users log in with their system
  password (PAM) or `~/.ssh/authorized_keys`.
* **Unprivileged daemon.** Radio input is parsed by `_bleterm`; root code is
  only OpenSSH's `sshd`, started per connection by systemd. The network SSH
  service can stay disabled.
* **Session lifetime = BLE connection.** On disconnect sshd gets EOF and the
  session receives SIGHUP, exactly like a dropped network SSH connection.
* One client at a time (v1).

Wire protocol: [docs/PROTOCOL.md](docs/PROTOCOL.md). Test guide:
[docs/TESTING.md](docs/TESTING.md).

## Install

```
sudo apt install ./ble-term_<version>_arm64.deb
```

The package creates the `_bleterm` user, generates the BLE SSH host key,
enables and starts `ble-term-session.socket` and `ble-term.service`, and
prints the host key fingerprint. Show it again with

```
ssh-keygen -lf /etc/ble-term/ssh_host_ed25519_key.pub
```

Logs: `journalctl -u ble-term -u 'ble-term-session@*'`.

Disable: `sudo systemctl disable --now ble-term.service ble-term-session.socket`.
Remove: `sudo apt purge ble-term`.

Requirements: BlueZ ≥ 5.55 (tested design targets 5.66 and 5.82), OpenSSH
server package (only the `sshd` binary is used), systemd. The Bluetooth adapter
must be powered (`bluetoothctl power on`); the daemon waits for it.

## Desktop client

`ble-term-connect` (in the same package, or `cargo build --release`) is an
OpenSSH ProxyCommand for Linux machines with BlueZ:

```
ble-term-connect --scan                       # list servers
ssh -o ProxyCommand='ble-term-connect AA:BB:CC:DD:EE:FF' -o HostKeyAlias=my-pi user@my-pi
```

## Development

```
cargo test                          # protocol unit tests
cargo build && tests/e2e-loopback.sh   # ssh ↔ simulated BLE ↔ sshd -i, no radio needed
```

`ble-term-loopback` runs both protocol ends over a simulated link (small MTU,
packet reordering) and is what the end-to-end test uses.
