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
| 2026-10-26 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.2-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | pass | — | pass | pass | pass | Upgrade over 0.2.1, then purge and clean install. With `ssh.service` stopped, BLE logins work (`/run/sshd` comes from the session unit). S14: an idle session stayed alive for 65 min. After about 6.6 h of heavy bench use the controller reported "hardware error 0x00" and stayed off until a reboot (see the device notes) |
| 2026-10-26 | Raspberry Pi Zero 2 W Rev 1.0 | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.2.2-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | pass | — | pass | pass | — | Upgrade, purge, clean install; without `Channels = 1` (the package turns ECRED off) |
| 2026-10-26 | Raspberry Pi 5 Model B Rev 1.1 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-2712 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.2-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | — | — | pass | pass | pass | Upgrade, purge, clean install; server role only. S14: a session with output every 30 s for 70 min, all lines received |
| 2026-10-26 | Flipper One F2 (prototype; Rockchip RK3576, MediaTek MT7921 USB Bluetooth) | FlipperOS (Debian 13 trixie), Minimal profile (no desktop) | 7.3.0-rc7 | 5.82-1.1 | 10.0p1 | 0.2.2-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | pass | — | pass | — | pass | pass | — | Upgrade, purge, clean install, reboot. 100 sessions back to back from the Pi Zero 2 W: 100/100. WirePlumber stopped for the runs |

Notes on devices:

* **Flipper One F2:** its Bluetooth controller (MT7921) stops advertising
  while a client is connected; it advertises again after the disconnect. The
  Raspberry Pi controllers keep advertising. Harmless with one session at a
  time.
* **Flipper One F2, Desktop and Minimal images:** WirePlumber publishes BLE
  MIDI; bench nodes with BlueZ's MIDI plugin then try to pair with it and
  the link drops, with the Flipper as server or as client (README, "Desktop
  client"). Stop WirePlumber for bench runs; phones are not affected.
* **Raspberry Pi 3 B+:** after hours of heavy bench use its controller
  (BCM43455 on UART) reported "hardware error 0x00" (twice so far); the
  reset failed and the adapter stayed off until a reboot. Not seen on the
  other boards.
* **Raspberry Pi Zero 2 W:** logins take 3 to 11 s over BLE (Pi 3 B+ and
  Pi 5: 3 to 4 s). The link runs at a 48.75 ms connection interval without
  data length extension, and the controller sends about one 27 byte packet
  per connection event; it shares its antenna with Wi-Fi, so the rate
  depends on 2.4 GHz conditions. Measured with `ble-term-connect` on
  another bench node (2026-10-26).

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
| 2026-10-26 | Pi 3 B+ → Flipper One F2 | 0.2.2-1 arm64 | 10/10 | |
| 2026-10-26 | Flipper One F2 → Pi Zero 2 W | 0.2.2-1 arm64 | 10/10 | Release candidate: 8/10 and 3 of 25 back-to-back sessions cut about 2 s in, by the server's deferred disconnect of the previous link (fixed before the release); release: 10/10 |
| 2026-10-27 | Raspberry Pi 3 Model B+ Rev 1.3 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-v8 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.3-1, 0.2.4-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | — | pass | pass | pass | — | pass | pass | S9: the first session stays up while a second client is told "server is busy" (before 0.2.3 it ended a second or two later). S14: an idle session stayed alive for 60 min. After about a day up and hours of bench use the controller reported "hardware error 0x00" again; fine after a reboot |
| 2026-10-27 | Raspberry Pi Zero 2 W Rev 1.0 | Raspberry Pi OS (Debian 12 bookworm), 64-bit, image 2026-10-06 | 6.12.109+rpt-rpi-v8 | 5.66-1+rpt2+deb12u2 | 9.2p1 | 0.2.3-1, 0.2.4-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | — | pass | pass | pass | — | pass | — | S9 as on the Pi 3 B+; before 0.2.3 the first session ended with a protocol error (BlueZ 5.66 passed the second client's writes into it) |
| 2026-10-27 | Raspberry Pi 5 Model B Rev 1.1 | Raspberry Pi OS (Debian 13 trixie), 64-bit, image 2026-10-06 | 6.18.50+rpt-rpi-2712 | 5.82-1.1+rpt2 | 10.0p1 | 0.2.3-1, 0.2.4-1 arm64 | `ble-term-connect` on another bench node | pass | pass | pass | — | — | — | pass | — | pass | — | — | — | pass | pass | S14: output every 30 s for 70 min, all lines received |
| 2026-10-26 | Pi Zero 2 W → Pi 3 B+ | 0.2.2-1 arm64 | 10/10 | |
| 2026-10-26 | Pi 3 B+ → Pi 5 | 0.2.2-1 arm64 | 10/10 | |
| 2026-10-26 | Pi Zero 2 W → Flipper One F2 | 0.2.2-1 arm64 | 100/100 | Sessions back to back (`echo`) |
| 2026-10-27 | Pi 3 B+ → Pi Zero 2 W | 0.2.3-1 arm64 | 4/4 | |
| 2026-10-27 | Pi Zero 2 W → Pi 3 B+ | 0.2.3-1 arm64 | 3/4, repeat 4/4 | One link loss 1 s into a session |
| 2026-10-27 | Pi 3 B+ → Pi 5, Pi Zero 2 W → Pi 5 | 0.2.3-1 arm64 | 4/4 each | |
| 2026-10-27 | Pi Zero 2 W → Pi 3 B+ | 0.2.3-1, 0.2.4-1 arm64 | 99/100 each | Sessions back to back. 0.2.3: one `ble-term-connect` found the previous link still going down ("GATT services have not been resolved"; fixed in 0.2.4). 0.2.4: one connection setup failed (`le-connection-abort-by-local`) |
| 2026-10-27 | Pi 3 B+ → Pi 5 | 0.2.4-1 arm64 | 100/100 | Sessions back to back |

Earlier builds failed sessions started right after a previous one (fixed in
0.1.3: sockets of a link that went down, leftovers before HELLO, two
clients on one link; fixed in 0.2.2: the server disconnected a client that
had already reconnected on a new link, about 10 s after its previous
session).

### blessh 0.2.0: short session cycles

`tests/e2e-cycles.sh -n 10 -m all` (10 sessions back to back and 10 pairs
of overlapping sessions, each running `echo`) on 2026-11-01 with
0.2.0~dev1: Debian packages on Linux, the Homebrew formula with `blessh
server enable` on the Mac. Machines: Raspberry Pi 3 B+ (Raspberry Pi OS
trixie, BlueZ 5.82), Raspberry Pi Zero 2 W (bookworm, BlueZ 5.66, no LE
Data Length Extension), ThinkPad X250 (Ubuntu 26.04, BlueZ 5.85), Intel
NUC 11 (Ubuntu 24.04, BlueZ 5.72), MacBook Pro with Apple silicon (macOS
15.7).

| Client → server | Result | Notes |
|---|---|---|
| Pi 3 B+ → Pi Zero 2 W | 20/20 | A first run had 3 failures while the Pi 3 B+ also served manual sessions to the Mac (link setup refused with ENOSYS, HCI 0x3e) |
| Pi Zero 2 W → Pi 3 B+ | 20/20 | |
| ThinkPad X250 → NUC 11 | 20/20 | |
| NUC 11 → ThinkPad X250 | 20/20 | |
| Pi 3 B+ → MacBook | 20/20 | Sessions reach sshd through the localhost launchd job |
| Pi Zero 2 W → MacBook | 20/20 | |
| MacBook → Pi 3 B+ | 20/20 | |
| MacBook → Pi Zero 2 W | 20/20 | The pair whose channel stalled before the channel handshake (0.1.3) |

### blessh 0.2.5: release builds

The packages and the Homebrew formula of the `blessh-v0.2.5` release, as
built by CI, on the same machines on 2026-11-01. Sessions as above
(`e2e-cycles.sh`, sequential and overlapping), then 200 KB through `cat`
in each direction.

| Client → server | Sessions | 200 KB down / up | Notes |
|---|---|---|---|
| Pi 3 B+ → Pi Zero 2 W | 20/20 | 109 s / 117 s | The pair that lost sessions to the grace timer before 0.2.3 |
| Pi Zero 2 W → Pi 3 B+ | 6/6 | 110 s / 93 s | |
| Pi 3 B+ → MacBook | 6/6 | 8 s / 11 s | |
| MacBook → Pi 3 B+ | 6/6 | 14 s / 18 s | |
| Pi Zero 2 W → MacBook | 6/6 | stops at 96 KiB / 126 s | Downloads to the Pi Zero 2 W stop (64 KiB in an earlier run); see backlog.md |
| MacBook → Pi Zero 2 W | 6/6 | 38 s / 216 s | |
| ThinkPad X250 → NUC 11 | 10/10 | 17 s / 39 s | |
| NUC 11 → ThinkPad X250 | 10/10 | 12 s / 15 s | |

Upgrades tested the same day: Debian packages 0.2.0 → 0.2.1 → … → 0.2.5
on all four Linux machines; Homebrew from the 0.2.1 formula (binary in
the Cellar) to 0.2.2 and on to 0.2.5, keeping the Bluetooth permission
and restarting the server by itself. Clean installs: purge and install on
three Linux machines; on the Mac `blessh server disable`, uninstall,
removal of `/etc/blessh` and the stable copy, then install and enable.

The Pi Zero 2 W's controller hung three times during the day under this
load (LE Set Random Address answered with EBUSY, then command timeouts);
only a reboot helped.

### blessh 0.2.6 and ConnectBot blessh r4

The `blessh-v0.2.6` release and ConnectBot blessh r4 from CI, on the same
machines and the Android 11 phone, on 2026-11-01. CLI as above with 5
sequential and 5 overlapping sessions per pair: 80/80; 200 KB both ways
on every pair, except the download from the MacBook to the Pi Zero 2 W,
which stopped at 64 KiB again. Phone, 3 sessions each to the Pi 3 B+ (by
name and by address), the Pi Zero 2 W and the MacBook: 12/12; the GATT
stream (Pi 3 B+ with `--no-l2cap`): 3/3 from the phone and from a Linux
client.

The reconnect race of 0.2.5 is gone: in three series of 8 phone sessions
to the Pi Zero 2 W the server dropped no link by its grace timer. Each
series still lost sessions, all from one cause: the controller stops
completing outgoing ACL packets in the middle of a session (no Number of
Completed Packets events from then on; the phone's link times out, the
Pi keeps the link and later fails HCI commands with "command tx
timeout"); the Pi 3 B+ did the same once. Only a reboot helps. It
happened 4 times in about 45 phone sessions that afternoon, after 6, 2
and 6 sessions on a freshly rebooted Pi Zero 2 W.

### The server list (after 0.2.6)

A CI build of `dev` with the list `blessh` shows without a server
(docs/client.md, "Choosing a server"), on 2026-11-01, on the Pi 3 B+,
the Pi Zero 2 W and the MacBook: Linux to Linux both ways, Linux to the
MacBook and the MacBook to the Pi 3 B+. Per pair, `blessh scan` in the
three formats, three sessions picked from the list by a key pressed on a
pseudo-terminal (`tests/pick-drive.py`), and a `blessh scan --timeout 0`
whose parent was killed: 12/12 picked sessions, no scan left running
after a pick or after its parent was killed (on the MacBook both of the
scan's processes); 11 of 12 eight-second scans named the expected server
(one on the MacBook missed it). A session picked from the list took 1 to
9 s from the MacBook and 5 to 18 s from a Linux client, the server's row
appearing 0.3 to 4 s after the scan started; a direct `blessh` session
from the Pi Zero 2 W to the Pi 3 B+ took 7.5 to 10 s.

### blessh 0.3.0 before release

The one package `blessh` and its installer, on 2026-11-02, on the Pi 3 B+
(BlueZ 5.82), the Pi Zero 2 W (5.66), the Linux laptop (5.85) and the
Linux PC (5.72), and the CI build of the macOS program on the MacBook.

* Installation: on all four Linux machines the old packages purged, then
  `install.sh --server` with a local build: the server on, its checks
  printed, and `install.sh` refused to install over the old packages
  before they were purged (dpkg's file conflict). Then the CI build over
  it: the upgrade restarted the running server and printed its status;
  running the installer again changed nothing and kept the server on.
  `disable` and `enable` round trips, `status` as a user (SSH checks
  marked as needing sudo) and as root.
* Visibility: the Linux servers no longer appear in the Android 11
  device picker (after Bluetooth restarted on the phone; it lists devices
  it found before), the MacBook does; with `--discoverable` the Pi 3 B+
  appears again. ConnectBot blessh r4 connects to the hidden servers
  (2/2 before, 9/9 to the three home servers after).
* CLI, five sequential and five overlapping sessions per pair, Linux to
  Linux at both locations and the MacBook to and from the Pi 3 B+: 60/60
  with the CI build (24/24 of three each with the local build); 200 KB
  each way on every pair, 101 to 120 s between the Pis, 8 to 17 s
  otherwise.
* The server list: 18/18 sessions picked by a key; 17 of 18 eight-second
  scans named the expected server (the MacBook missed the Pi 3 B+ once,
  as before).

### blessh 0.3.0 and ConnectBot blessh r4: long session

Six hours on 2026-11-02 with the published 0.3.0 (installed with
`install.sh --server` and `brew upgrade`) on two clusters: an arm64 board
with a MediaTek controller (Debian 13, BlueZ 5.82), the Linux laptop
(Realtek, 5.85) and the Linux PC (Intel, 5.72); the Pi 3 B+, the Pi Zero
2 W, the MacBook and the Android 11 phone with r4. Scenarios: sessions
with gaps of 0–70 s around the 30 s grace period, 75-minute interactive
sessions with bulk transfers next to them, 30-minute silent sessions,
several sessions per link and the session limit, clients killed in the
middle, adapter power cycles and Bluetooth restarts on servers, port
forwarding, the phone killed or its Bluetooth switched off mid-session.

* No session dropped on its own; 303 of 322 sessions with gaps, every
  failure explained (controller failures on the Pis, a stale cached
  device). Killed clients: the server ended the session within 2 s and
  the next session worked, 45/45. Phone: 39/45 with gaps (1 failed link
  setup, 5 during the Pi Zero 2 W's TX stall), 20/20 in the other phone
  scenarios.
* Systematic problems found, to fix in code: after the adapter is
  switched off and on, a server on a controller with extended advertising
  stays invisible until restarted; a Linux client that exits disconnects
  a link the peer made, ending the peer's session to this machine's
  server, and neither client can open a session over such a link; the
  Linux client may select a stale cached device of the same name; a
  server with extended advertising takes one client device at a time;
  typing waits up to 4 s while another session on the link uploads.
* The Pi 3 B+ controller failed twice (hardware error, HCI reset timing
  out) and the Pi Zero 2 W stalled once; each needed a reboot.

### blessh 0.3.1 before release

The fixes for the long session's findings, CI builds on the same
machines on 2026-11-02 (the phone not available):

* Adapter switched off and on: the servers came back within 3–18 s every
  time, 24/24: `bluetoothctl power off/on` with 1 and 5 s off and rfkill
  with 2 s off on the three extended-advertising controllers, power
  cycles on the Pis.
* A Linux client to a machine connected to it: the session the other
  machine holds survives, 15/15 (the Linux PC and laptop both ways, the
  arm64 board and the laptop both ways, the Pi 3 B+ to the MacBook). The
  client's own session over that link still fails except to the MacBook
  (backlog).
* Sessions with gaps: 77/84; the failures were the Pi Zero 2 W's TX stall
  (6) and one link lost. Killed clients: 2 cases, next sessions 6/6.

## Clients

| Date | Client | OS | App build | Server | Result | Notes |
|---|---|---|---|---|---|---|
| 2026-10-24 | Google Pixel 8 Pro | GrapheneOS (Android 17) | connectbot-ble 3 (adc47dc) | Raspberry Pi 3 Model B+ | pass | Client uses rotating private addresses |
| 2026-10-25 | Google Pixel 8 Pro | GrapheneOS (Android 17) | ble-r1 … ble-r5 | all bench servers | pass | Scan, login, compression, rotation, background, reconnect, CLOSE on server restart; pairing request detected with EATT turned on (ble-r4 showed the hint only in the app log) |
| 2026-10-26 | Android 11 phone | Android 11 | ble-r3 … ble-r5 | bench servers | pass | Scan finds nothing while location is off (Android holds results back); ble-r5 says so in the scan dialog, not yet checked on the phone. Up to ble-r2 the app crashed after login on servers with an ECDSA host key (sshlib on Android 12 and older); worked around since ble-r3 |
| 2026-10-26 | Android 8 phone | Android 8 | ble-r3 | bench servers | pass | Scan and BLE login |
| 2026-10-27 | Android 11 and Android 8 (Huawei P9) phones | Android 11, Android 8 | ble-r6 | bench servers | pass | Login without the client's former sshlib workaround (sshlib 0.6.5); the Android 8 phone also finds servers with location off. Two phones at once: the second was told the server is busy, but the first one's session ended a few seconds later (BlueZ, see docs/protocol.md; servers from 0.2.3 avoid it) |
| 2026-11-01 | Google Pixel 2 XL | Android 11 | ConnectBot blessh r3 | Pi 3 B+ (by name and by address), Pi Zero 2 W, MacBook; blessh 0.2.5 | pass | L2CAP channel to all; 12/12 sessions, with the app stopped between them. A further 5 to the Pi Zero 2 W: 3/5, lost to the reconnect race in backlog.md. GATT stream (server with `--no-l2cap`): 3/3. Upgrade r1 → r2 → r3 keeps hosts and keys |
| 2026-11-01 | Google Pixel 2 XL | Android 11 | ConnectBot blessh r4 | the same; blessh 0.2.6 | pass | 12/12; no session cut by the server's grace timer. A third to all connects lose 10 s to a GattService fault on this phone (backlog.md); the app recovers by itself. Saved hosts for all three servers |
