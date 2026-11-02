# Backlog

Ideas worth doing, not scheduled yet, and open points from the blessh
rework (2026-10).

## macOS

* The GATT stream on macOS (client and server): CoreBluetooth
  notifications and writes without response, gated on
  `canSendWriteWithoutResponse`; until then macOS needs L2CAP peers.
* Wake the channel thread on writes instead of its 10 ms tick (a run loop
  source), and bound sessions per device as on Linux.
* Logs of BLE sessions in sshd: today sshd logs them as coming from
  127.0.0.1 (macOS) or "UNKNOWN" (Linux), and the server's own log joins
  them by the source port (macOS) or the session unit (Linux). Possible
  later, if wanted despite touching the administrator's settings:
  `LogLevel VERBOSE` above the `Include` in `/etc/blessh/sshd_config`
  (sshd then logs the local port, which marks BLE sessions), and the
  server's lines in the unified log under a subsystem `org.blessh`. The
  criterion so far: as little change to the system and as few settings as
  possible.
* A signed `.pkg` for Macs without Homebrew.
* blessh as an `.app` bundle: macOS keys the Bluetooth permission of a
  program outside a bundle to its path (and its signature), which is why
  the formula runs blessh from a copy at a stable path
  (`var/lib/blessh/blessh`); a bundle would be keyed by its identifier
  instead, and would give its own entry in the Bluetooth and Login Items
  settings (SMAppService) and a Homebrew cask. Needs the bundle layout,
  SMAppService registration, and for other machines a Developer ID
  signature and notarization.
* Server names on macOS come from HostName or ComputerName at every
  start, without the " (N)" macOS appends on a network conflict; a name
  the user really ends in " (N)" loses it as well. A name like Linux's
  `/etc/hostname` that macOS never changes does not exist; watch for
  other renames (LocalHostName gets "-N", unused here).
* Throughput over the channel is low (about 10 KB/s per direction
  between a Mac and Linux, 10 to 30 KB/s between Linux machines): look at
  the channel thread's buffers and tick, the connection interval the
  central asks for, and the L2CAP credits.
* Verify on hardware: CoreBluetooth against BlueZ over the channel, the
  permission prompt for a LaunchAgent, sleep and logout behaviour.

## Links

* The Linux server drops a link thirty seconds after its last channel
  closed unless the client read INFO meanwhile. A client that opens a
  channel on that link within the seconds the disconnect takes to reach
  the radio (up to three on a Pi Zero 2 W) still loses its new session.
  Clients rarely keep an idle link that long (ConnectBot blessh closes
  it when its session ends). Closing the gap needs a way to
  cancel or hold the disconnect. To reproduce: a client that keeps the
  link idle (`bluetoothctl connect` plus a script that reads INFO and
  opens a channel), opening the channel 29 to 33 s after the previous
  one closed, in half-second steps; a hit shows in the server's journal
  as `no channel for 30s, disconnecting` followed within seconds by the
  new session ending with `link closed on this side`.
* The Linux client that joins a link another blessh made takes the PSMs
  from the lock file and reads no INFO; it should read INFO too
  (protocol.md, "Lifetime").
* The Broadcom controllers of the Pi Zero 2 W and the Pi 3 B+ sometimes
  stop completing outgoing ACL packets in the middle of a phone session
  and need a reboot (hardware.md, "blessh 0.2.6"). Worth finding out
  whether the phone's 7.5 ms connection interval (the app asks for high
  priority) makes it more likely, and whether the server can notice it
  (HCI command timeouts in the kernel log) and say so. The long test of
  2026-11-02 (hardware.md) added two more ways the Pi 3 B+ controller
  fails: a hardware error with HCI Reset timing out, once right after a
  session as client while its own server advertised, once after power
  cycles and a bluetoothd restart that left the adapter unpowerable
  ("Failed to set mode: Authentication Failed").
* A client cannot open a session over a link the peer made (the peer
  connected to this machine's own server): the Linux client gets EBUSY
  for its ATT channel, falls back to bluetoothd after 5 s, and bluetoothd
  has not resolved the services of a peer that connected to it ("device
  does not provide the blessh service"); the macOS client fails the same
  way. Two machines that log in to each other at the same moment hit it.
  Ideas: open the L2CAP channel directly with the PSM from the channel
  cache, or have bluetoothd resolve the peer's services and read INFO
  through it.
* Typing waits up to 4 s while another session on the same link uploads
  (laptop with a Realtek controller to the arm64 board: 300 KB up in 77
  s; between the arm64 board and the PC both ways stay fast). Measure
  first (btmon during an upload); then perhaps cap the bytes in flight
  per channel or shrink the socket send buffer, so that channels on one
  link interleave.
* Large downloads from a macOS server to a Linux client on a controller
  without LE Data Length Extension (Pi Zero 2 W) stop at 64 or 96 KiB;
  uploads and interactive sessions work.

## Distribution

* An APT repository (for example on GitHub Pages), so that Debian and
  Raspberry Pi OS users get updates with `apt upgrade` instead of
  downloading packages from Releases; signed, which the installer script
  cannot be. The installer would then add the repository instead of
  installing a downloaded package, and the package could recommend
  `openssh-server` instead of only suggesting it (now the server's
  machine has to install it by hand, to keep it off clients, since apt
  installs recommended packages).
* A package for 32-bit Raspberry Pi OS (`armhf`); the installer refuses it
  for now.

## Clients

* ConnectBot blessh on Android 11 (Pixel 2 XL): in a third to all of
  the connects, depending on the day, the system's GattService reports
  the connection to the app but has no record of it, and drops the app's
  first GATT operation (`discoverServices() - No connection for …` in
  logcat); no callback ever comes. The app reopens its GATT client on the
  same link after its 10-second operation timeout, which always worked,
  so the connect takes 10 s longer. Delaying the operation by a second
  did not help. Seen with Pi 3 B+ and Pi Zero 2 W servers alike; not
  known on other Android versions. A shorter discovery timeout would
  halve the cost; the cause is outside the app.
* ConnectBot blessh: when link setup fails (GATT status 133, HCI 0x3e)
  the app scans again instead of retrying the connect, and gives up if
  the scan does not hear the server within 15 s (1 of 53 sessions in the
  2026-11-02 test). The Linux client retries link setup three times;
  the app could retry `connectGatt` once before scanning.
* `blessh`'s list (docs/client.md, "Choosing a server"):
  * Servers it does not show: one this machine is connected to already,
    and one whose controller does not advertise while another client is
    connected; `blessh <name>` still reaches the first.
  * On Linux the list shows the name bluetoothd has for a device, which
    after an earlier connection may be the server's GAP device name
    (BlueZ's `Name` in `main.conf` or the pretty host name) rather than
    the advertised one. The proxy still reaches the device chosen, but
    the host key is pinned under the wrong name (a later `blessh <real
    name>` asks for it again) and duplicate names are judged by it. The
    name in the advertisement's service data would fix it (see "Server
    and protocol").
  * The list's scan takes the default adapter; a `BLESSH_ADAPTER` for the
    scan and the proxy alike would choose another.
  * Marks for servers whose host key is known, or that were used
    recently: a small command or option, not in the list itself.
* A server name that is also a `Host` in `~/.ssh/config` gets that
  entry's settings over BLE too (`User` and `IdentityFile` may be
  wanted), and with `ControlMaster auto` and a `ControlPath` built from
  `%h`, `blessh <name>` may reuse the network connection's master
  silently. Possible: `-o ControlPath=none` unless the user sets one, and
  a tag for `Match tagged` (OpenSSH 9.4 and later; Debian 12 has 9.2).
* Shell completion of server names from those seen recently.
* `blessh doctor`: adapter, Bluetooth permission, BlueZ version, EATT, the
  server's state, in one report.
* iOS app, sharing the CoreBluetooth code with the macOS client.
* Windows client over WinRT GATT (no L2CAP API): `blessh proxy` only.
* Web client (Web Bluetooth + WASM SSH): origin trust and supply chain
  still to be decided; `web/` is reserved for it.

## Server and protocol

* Root cause of the channel stall between CoreBluetooth and Linux on
  controllers without LE Data Length Extension (Pi Zero 2 W, 27-byte LE
  buffer, MPS 23): CoreBluetooth sends 19-byte K-frames and stops in the
  middle of an SDU while Linux still counts credits for it; Linux returns
  credits only when its count reaches zero, because with MPS 23
  `l2cap_publish_rx_avail` always falls back to "one SDU" (the sk_buff
  overhead estimate exceeds the buffer). Seen in both directions; the
  channel handshake detects it and the second channel (server MTU 64) or
  a client MTU of 64 avoid it, at about 1 KB/s. Worth a kernel report with
  traces, and a look at whether CoreBluetooth or Linux breaks the spec.
* macOS client and a Pi Zero 2 W server: now and then CoreBluetooth
  reports the manager as powered off for a moment (often as another
  blessh process exits) and drops the peripheral, which ends the session
  (1 to 2 in 20 short sessions); not seen with other servers.
* A faster fallback than MTU 64 for such links (MTU 128 worked 2 of 3
  times); the right value may depend on the sender's K-frame size.

* A server whose controller uses extended advertising serves one client
  device at a time: the kernel does not enable the advertising set again
  after a connection ended it, so a second device finds nothing until
  the first disconnects (6/6 on the arm64 board; legacy advertising on
  the Pi 3 B+ does take a second device). The controller can: a second
  advertisement registered by hand let another device connect. Planned:
  register the advertisement again when a client connects, on adapters
  whose MaxAdvLen is above 31 (extended advertising).
* Verify on real radios what btvirt cannot: phones, CoreBluetooth against
  BlueZ, and the Bookworm residual risk (a phone that answers bluetoothd's
  legacy fallback request on PSM 0x27 with "insufficient encryption" would
  still be asked to pair).
* PSM in the advertisement's service data, so Android clients can open
  the channel without a GATT round trip.
* Selection by name on Linux: bluetoothd may show the GAP device name
  instead of the advertised one; the client then asks servers whose name
  does not match for their INFO after a few seconds, which connects to
  every server in range. The name in the advertisement's service data
  would avoid that.
* `blessh scan` on Linux lists only devices whose advertised service
  UUID bluetoothd currently knows; it forgets them when a link drops,
  until it hears the device again.
* Session resumption over a new link (would need a layer above raw SSH).
* GATT stream with several sessions on BlueZ ≥ 5.69 (per-device
  sockets), if a GATT-only client ever needs it.
* The vhci test with `DAEMON_USER` (D-Bus policy) in CI, which needs
  `hci_vhci` on the runner.

## Repository

* `tests/e2e-vhci.sh` on the bench: btvirt (BlueZ 5.72, Linux 6.17)
  delivers no advertisements between its virtual controllers any more,
  even with older blessh and after a reboot, so the test only passes when
  bluetoothd already knows the server. Once it works again, add steps
  for a server that advertises again after its adapter is switched off
  and on, and for a client that leaves a peer's link up.
