# Testing ble-term v1 on a Raspberry Pi 5

## 1. Server

```
sudo apt install ./ble-term_0.1.0-<rev>_arm64.deb
```

Expected output ends with the BLE SSH host key fingerprint
(`256 SHA256:… ble-term@<host> (ED25519)`). Note it; the phone shows the same
fingerprint on first connect.

Checks:

```
systemctl status ble-term.service ble-term-session.socket
journalctl -u ble-term -n 20        # "ready (BlueZ GATT application and advertisement registered)"
bluetoothctl show                   # Powered: yes
```

If the adapter is off: `sudo rfkill unblock bluetooth; bluetoothctl power on`.
The daemon advertises the host name; set another one with
`BLE_TERM_ARGS="--name my-pi"` in `/etc/default/ble-term` and
`sudo systemctl restart ble-term`.

## 2. Android (ConnectBot BLE build)

1. Install the APK (`adb install -r connectbot-ble-*.apk` or open it on the
   phone). It installs next to the Play Store ConnectBot (separate package id).
2. Hosts → **+** → protocol **ble** → **Scan** → pick the Pi (or type the
   Bluetooth address). Enter the user name, save.
3. Grant "Nearby devices" when asked.
4. Tap the host. Accept the host key after comparing the fingerprint with the
   one printed on the Pi, then log in with the Pi user's password (or add a
   ConnectBot public key to `~/.ssh/authorized_keys`).

Things worth trying: `htop`, `vim`, `nano`, `nmtui`, `tmux`, rotating the
phone (resize), long output (`dmesg`, `cat /usr/share/common-licenses/GPL-3`)
followed by Ctrl+C, Bluetooth off on the phone in the middle of a session
(the session must end on the Pi: `who`, `journalctl -u 'ble-term-session@*'`).

## 3. Linux desktop client (optional)

On any Linux machine with BlueZ and Bluetooth:

```
sudo apt install ./ble-term_0.1.0-<rev>_amd64.deb   # or arm64; only ble-term-connect is needed
sudo systemctl disable --now ble-term.service ble-term-session.socket   # client-only machine
ble-term-connect --scan
ssh -o ProxyCommand='ble-term-connect <pi-address>' -o HostKeyAlias=pi-ble <user>@pi-ble
```

## What to report

* `journalctl -u ble-term -u 'ble-term-session@*' --since '10 min ago'`
* `bluetoothctl --version`, `uname -r`, OS release
* Android version and phone model; for app problems `adb logcat -d | grep -i -e BleTransport -e ConnectBot`
