# Verified hardware

Server devices and clients on which ble-term was tested with the procedure
in [TESTING.md](TESTING.md). Case IDs refer to that file: `pass`, `fail`,
`—` = not run. Add a row for every new device, OS release or major BlueZ
version.

## Servers

| Date | Device | OS | Kernel | BlueZ | OpenSSH | Package | Client | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S8 | S9 | S10 | S11 | S12 | S13 | S14 | Notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-24 | Raspberry Pi 3 Model B+ Rev 1.3 (rns-3) | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.1.0-4 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | — | pass | — | — | pass | — | — | — | — | Bluetooth was soft-blocked by rfkill after install; fixed with `rfkill unblock bluetooth`, survives reboot |
| 2026-10-24 | Raspberry Pi 3 Model B+ Rev 1.3 (rns-3) | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.1.2-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | pass | pass | — | — | — | — | pass | pass | — | Upgrade from 0.1.0-4 kept the host key; reconnect right after `exit` is immediate; with the 400 ms advertising interval connecting is noticeably faster |
| 2026-10-24 | Raspberry Pi Zero 2 W Rev 1.0 (rpz2w) | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.1.2-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | pass | pass | — | — | — | pass | pass | pass | — | Needs `Channels = 1` in `/etc/bluetooth/main.conf` (BlueZ 5.66 EATT: pairing request and dropped connections otherwise). BlueZ 5.66 ignores the advertising interval (stays 1.28 s). Raspberry Pi OS **trixie** could not join any Wi-Fi on this board (802.11 authentication timeout, also with a phone hotspot); bookworm works |

## Clients

| Date | Client | OS | App build | Server | Result | Notes |
|---|---|---|---|---|---|---|
| 2026-10-24 | Google Pixel 8 Pro | GrapheneOS (Android 17) | connectbot-ble 3 (adc47dc) | rns-3 | pass | Client uses rotating private addresses |
