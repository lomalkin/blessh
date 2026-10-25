# Verified hardware

Server devices and clients on which ble-term was tested with the procedure
in [testing.md](testing.md). Case IDs refer to that file: `pass`, `fail`,
`—` = not run. Add a row for every new device, OS release or major BlueZ
version.

## Servers

| Date | Device | OS | Kernel | BlueZ | OpenSSH | Package | Client | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S8 | S9 | S10 | S11 | S12 | S13 | S14 | Notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-24 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.1.0-4 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | — | pass | — | — | pass | — | — | — | — | Bluetooth was soft-blocked by rfkill after install; fixed with `rfkill unblock bluetooth`, survives reboot |
| 2026-10-24 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.1.2-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | pass | pass | — | — | — | — | pass | pass | — | Upgrade from 0.1.0-4 kept the host key; reconnect right after `exit` is immediate; with the 400 ms advertising interval connecting is noticeably faster |
| 2026-10-24 | Raspberry Pi 5 Model B Rev 1.1 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-2712 | 5.82-1.1+rpt2 | 10.0p1 | 0.1.2-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | — | pass | — | — | — | — | — | — | — | Fresh install of 0.1.2-1; Bluetooth was not blocked |
| 2026-10-24 | Raspberry Pi Zero 2 W Rev 1.0 | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.1.2-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | pass | pass | — | — | — | pass | pass | pass | — | Needs `Channels = 1` in `/etc/bluetooth/main.conf` (BlueZ 5.66 EATT: pairing request and dropped connections otherwise). BlueZ 5.66 ignores the advertising interval (stays 1.28 s). Raspberry Pi OS **trixie** could not join any Wi-Fi on this board (802.11 authentication timeout, also with a phone hotspot); bookworm works |
| 2026-10-25 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.1.3~ci37-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | — | pass | pass | — | — | — | pass | pass | — | — | — | — | — | — | Release candidate of 0.1.3 under the D-Bus policy |
| 2026-10-25 | Raspberry Pi Zero 2 W Rev 1.0 | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.1.3~ci37-1 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | — | pass | pass | — | — | — | pass | pass | — | — | — | — | — | — | Release candidate of 0.1.3 under the D-Bus policy; the installer warns about BlueZ < 5.76 |
| 2026-10-25 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.0-1 arm64 | `ble-term-connect` on another bench node | pass | — | pass | — | — | — | pass | pass | — | pass | pass | pass | pass | — | Clean install after purge. Upgrade from 0.1.3: the installer announces the host key change, the client refuses the old pin, the old key stays in `/etc/ble-term` until purge, an active session ends. Over the radio: password login refused (system policy), a `sshd_config.d` drop-in and a setting above the `Include` apply to BLE, the latter not to network SSH; `_bleterm` denied `StartDiscovery` |
| 2026-10-25 | Raspberry Pi 5 Model B Rev 1.1 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-2712 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.0-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | — | — | pass | — | — | Clean install after purge; server role only (see Bench to bench) |
| 2026-10-25 | Raspberry Pi Zero 2 W Rev 1.0 | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.2.0-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | — | pass | pass | — | — | Clean install after purge; `Channels = 1` kept. S11: the first connection after the power cycle timed out without reaching the daemon, the next ones worked |
| 2026-10-25 | Flipper One F2 (prototype; Rockchip RK3576, MediaTek MT7921 USB Bluetooth) | FlipperOS (Debian 13 trixie), Minimal profile (no desktop) | 7.3.0-rc7 | 5.82-1.1 | 10.0p1 | 0.2.0-1, 0.2.1~ci45 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | — | pass | — | — | pass | — | pass | pass | — | Profiles switch with a systemd soft-reboot, which does not bring up `bluetooth.target` (the adapter is already there): with 0.2.0 neither bluetoothd nor ble-term started; since 0.2.1 ble-term starts from `multi-user.target` and starts bluetoothd through D-Bus activation (soft-reboots into the same profile pass). The rfkill block set in the Desktop profile survives a soft-reboot (same kernel) |
| 2026-10-25 | Flipper One F2 (prototype; Rockchip RK3576, MediaTek MT7921 USB Bluetooth) | FlipperOS (Debian 13 trixie), Desktop profile (KDE Plasma 6, bluedevil and blueman) | 7.3.0-rc7 | 5.82-1.1 | 10.0p1 | 0.2.0-1, 0.2.1~ci47 arm64 | Google Pixel 8 Pro, GrapheneOS (Android 17) | pass | pass | pass | — | — | — | — | — | — | fail | — | pass | pass | — | Works until the next reboot from KDE: bluetoothd stops before the desktop session, bluedevil records the adapter as off and blocks it with rfkill at the next login (systemd-rfkill then keeps the block). Not a ble-term problem; fixed in the system by stopping user sessions before bluetoothd or by setting bluedevil to enable Bluetooth at login. ble-term itself starts after the reboot and reports the block |
| 2026-10-25 | PC, Intel Wireless-AC 9460/9560 Bluetooth (USB 8087:0aaa) | Ubuntu 24.04 LTS, x86_64 | 6.17.0-14-generic | 5.72-0ubuntu5.5 | 9.6p1 | 0.1.3-1 amd64 | ConnectBot BLE (Android) | pass | pass | pass | pass | pass | pass | pass | — | — | — | — | — | — | — | First x86_64 server. Checked by the operator: install and normal use; reboot not tested. BlueZ 5.72 has EATT off by default; the installer warns about BlueZ < 5.76 |

| 2026-10-25 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.1~ci52-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | pass | — | pass | pass | — | Upgrade over 0.2.0 (unit files moved to `/usr/lib` and still there), then purge and clean install; installer check table |
| 2026-10-25 | Raspberry Pi Zero 2 W Rev 1.0 | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.2.1~ci52-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | pass | — | pass | pass | — | Without `Channels = 1`: the package turns ECRED off (also after reboot); phone gets no pairing request. Upgrade, purge, clean install |
| 2026-10-25 | Raspberry Pi 5 Model B Rev 1.1 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-2712 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.1~ci52-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | — | — | pass | pass | — | Upgrade, purge, clean install; server role only |
| 2026-10-25 | Flipper One F2 (prototype; Rockchip RK3576, MediaTek MT7921 USB Bluetooth) | FlipperOS (Debian 13 trixie), Minimal profile (no desktop) | 7.3.0-rc7 | 5.82-1.1 | 10.0p1 | 0.2.1~ci52-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | pass | — | pass | pass | — | Reboot and soft-reboot pass. CLOSE on shutdown: the client gets "server is shutting down". WirePlumber publishes BLE MIDI, which makes BlueZ clients try to pair and hang (README, "Desktop client"); stopped for the client runs |
## Bench to bench

`tests/e2e-bench.sh` between two bench machines (tier 2 in
[testing.md](testing.md)): `ble-term-connect` on the first, the server on
the second.

| Date | Client → server | Package | Result | Notes |
|---|---|---|---|---|
| 2026-10-25 | Pi 3 B+ (trixie, BlueZ 5.82) → Pi Zero 2 W (bookworm, BlueZ 5.66) | 0.1.3~ci37 arm64 | 10/10 | 30 KB each way, 1.0–1.3 KB/s (48.75 ms connection interval, no data length extension on these controllers) |
| 2026-10-25 | Pi Zero 2 W → Pi 3 B+ | 0.1.3~ci37 arm64 | 8/10, repeat pass | The first two cases right after the previous direction lost the link before GATT (no session on the server); repeated: 6/6 sessions back to back |
| 2026-10-25 | Pi 5 → Pi 3 B+, Pi 5 → Pi Zero 2 W | 0.1.3~ci33 arm64 | fail | Connection setup fails (`le-connection-abort-by-local`); the server role works. This board also has an unresolved 5 GHz Wi-Fi problem; not treated as a ble-term bug |
| 2026-10-25 | Pi 3 B+ → Pi Zero 2 W | 0.2.0-1 arm64 | 10/10 | 0.9–1.3 KB/s; the pinned host key is the server's system key |
| 2026-10-25 | Pi Zero 2 W → Pi 3 B+ | 0.2.0-1 arm64 | 9/10, repeat 10/10 | First session after switching direction lost the link right after login, as with 0.1.3 |
| 2026-10-25 | Pi 3 B+ → Pi 5 | 0.2.0-1 arm64 | 10/10 | 5.2–5.6 KB/s (2.6 KB/s with write requests) |
| 2026-10-25 | Pi 3 B+ → Flipper One F2 | 0.2.1~ci52 arm64 | 10/10 | With WirePlumber stopped on the server (BLE MIDI) |
| 2026-10-25 | Flipper One F2 → Pi Zero 2 W | 0.2.1~ci52 arm64 | 9/10 | "5 sessions back to back": one link loss, as on earlier first runs after a direction change |
| 2026-10-25 | Pi Zero 2 W → Pi 3 B+ | 0.2.1~ci52 arm64 | 9/10 | All sessions pass; the failed check is the client's EATT warning, which ignored the ECRED switch (fixed before the release) |
| 2026-10-25 | Pi 3 B+ → Pi 5 | 0.2.1~ci52 arm64 | 10/10 | 5.4–6.1 KB/s |

Earlier builds failed sessions started right after a previous one (fixed in
0.1.3: sockets of a link that went down, leftovers before HELLO, two
clients on one link).

## Clients

| Date | Client | OS | App build | Server | Result | Notes |
|---|---|---|---|---|---|---|
| 2026-10-24 | Google Pixel 8 Pro | GrapheneOS (Android 17) | connectbot-ble 3 (adc47dc) | Raspberry Pi 3 Model B+ | pass | Client uses rotating private addresses |
