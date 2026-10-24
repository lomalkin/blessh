# Testing ble-term v1 on a Raspberry Pi 5

## What has been verified before hardware tests

| Check | How | Result |
|---|---|---|
| Protocol state machine (Rust) | `cargo test` | pass |
| Protocol state machine (Kotlin, shared test vectors) | `BleStreamLinkTest` in the ConnectBot fork CI | pass |
| OpenSSH ↔ BleStream ↔ `sshd -i`, MTU 20/185/512, packet reordering, 300 KB each way, PTY | `tests/e2e-loopback.sh` (CI) | pass |
| Real BlueZ 5.66 (Bookworm build) GATT path: advertising, discovery, AcquireWrite/AcquireNotify, EATT, 100 KB each way, PTY, reconnects, session ends when the client disappears; daemon unprivileged under Debian's D-Bus policy | `tests/e2e-vhci.sh` with two `btvirt` virtual controllers | pass |
| `/etc/ble-term/sshd_config` | `sshd -t` / `sshd -T` | valid |
| `ble-term.service` hardening | `systemd-analyze security` | exposure 1.8 (OK) |

Not verified yet: a real radio (CYW43455 on the Pi 5), BlueZ 5.82, an Android
phone, the installed `.deb` with systemd and PAM.

## 1. Server

```
sudo apt install ./ble-term_0.1.0-<rev>_arm64.deb
```

The output ends with the BLE SSH host key fingerprint
(`256 SHA256:… ble-term@<host> (ED25519)`). Note it; ConnectBot shows the same
fingerprint on first connect.

Checks:

```
systemctl status ble-term.service ble-term-session.socket
journalctl -u ble-term -n 20        # "ready (BlueZ GATT application and advertisement registered)"
bluetoothctl show                   # Powered: yes
```

If the adapter is off: `sudo rfkill unblock bluetooth; bluetoothctl power on`.
The daemon advertises the host name. To change it, set
`BLE_TERM_ARGS="--name my-pi"` in `/etc/default/ble-term` and run
`sudo systemctl restart ble-term`.

## 2. Android (ConnectBot BLE build)

1. Install `connectbot-ble-<n>-<sha>.apk`. It is a debug build with package id
   `org.connectbot.debug`, so it installs next to a Play Store ConnectBot. If
   Android refuses to update an earlier test build, uninstall that build first.
2. Hosts → **+** → protocol **ble** → **Scan for Bluetooth LE servers**, allow
   "Nearby devices", pick the Pi. The Bluetooth address and the nickname are
   filled in. Enter the Pi user name and save. Use the scan button at least once
   even if you type the address: it is where the permission is requested.
3. Tap the host. Compare the host key fingerprint with the one from step 1 and
   accept it, then log in with the Pi user's password. Keys work too: add a
   ConnectBot public key to `~/.ssh/authorized_keys` on the Pi.

The terminal prints `Connecting to <address> over Bluetooth LE` and
`Bluetooth LE link up (MTU …)` before the SSH login.

Things worth trying:

* `htop`, `vim`, `nano`, `nmtui`, `tmux`
* rotating the phone (window resize)
* long output (`dmesg`, `cat /usr/share/common-licenses/GPL-3`) followed by Ctrl+C
* `exit`, then reconnecting
* Bluetooth off on the phone, or walking out of range, in the middle of a
  session. The session must end on the Pi: check with `who` and
  `journalctl -u 'ble-term-session@*'`

## 3. Linux desktop client (optional)

On any Linux machine with BlueZ and Bluetooth:

```
sudo apt install ./ble-term_0.1.0-<rev>_amd64.deb   # or arm64; only ble-term-connect is needed
sudo systemctl disable --now ble-term.service ble-term-session.socket   # client-only machine
ble-term-connect --scan
ssh -o ProxyCommand='ble-term-connect <pi-address>' -o HostKeyAlias=pi-ble <user>@pi-ble
```

## Troubleshooting knobs

* `--readvertise` in `BLE_TERM_ARGS`: if the Pi stops being discoverable after
  the first session, this re-registers the advertisement after each session.
  It is off by default because it broke reconnects in the virtual-controller
  tests.
* `RUST_LOG=debug` in `/etc/default/ble-term` (as an extra line) for verbose
  daemon logs.

## What to report

* `journalctl -u ble-term -u 'ble-term-session@*' --since '10 min ago'`
* `bluetoothctl --version`, `uname -r`, `/etc/os-release`
* Android version and phone model. For app problems:
  `adb logcat -d | grep -i -e BleStream -e "BLE " -e ConnectBot`
