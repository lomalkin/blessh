# Architecture

blessh carries SSH over Bluetooth Low Energy. This document fixes the
principles the design follows, the platforms it serves and how the pieces
of this implementation fit together. What every client and server must do
on the wire, whoever writes it, is specified in
[protocol.md](protocol.md).

## Principles

1. **Simple discovery.** A server advertises one service UUID and its
   name; a client scans for the UUID and picks a server by name or by
   Bluetooth address. No IP addresses, no DNS, no network.
2. **No pairing.** Nothing requires Bluetooth encryption or bonding, on
   any platform. Pairing dialogs are a defect to be fixed, never a step.
3. **Security is SSH's.** The link is open; confidentiality,
   authentication and authorization come from sshd and the client's host
   key pinning. The server never adds its own trust material (no key
   fingerprints in advertising data or characteristics: anyone can forge
   them), and never bypasses the system's SSH policy.
4. **Address by what the radio gives.** A server is selected by its
   advertised name or its Bluetooth address, both stable without a
   network. Platforms that hide addresses (Apple, web) select by name.
5. **Nothing to configure.** Installing the server package is enough; the
   client needs no settings. Options exist for the rare case, with sane
   defaults.
6. **Coexistence.** The server changes no Bluetooth stack configuration
   and takes no adapter over: other Bluetooth devices and services on the
   same machine keep working, and the server keeps working next to them.
7. **Compatibility and reliability over speed.** A transport that works
   everywhere and tells the truth about link loss beats a faster one with
   corner cases.

## Platforms

| Platform | Server | Client | Transport |
|---|---|---|---|
| Linux, BlueZ ≥ 5.55 | yes (this repository) | yes (this repository) | L2CAP channel, GATT stream as fallback |
| macOS | yes (this repository; user session only, since macOS allows BLE to logged-in users; L2CAP clients only) | yes (this repository; L2CAP only) | L2CAP channel |
| Android 10 and later | no | [ConnectBot blessh](https://github.com/lomalkin/connectbot-blessh) | L2CAP channel, GATT stream |
| Android 7 to 9 | no | ConnectBot blessh | GATT stream |
| iOS 11 and later | no | planned | L2CAP channel, GATT stream |
| Windows | no | planned | GATT stream (no public L2CAP API) |
| Web (Chrome, Edge) | no | planned | GATT stream (no L2CAP in Web Bluetooth) |

Every combination works without pairing. Two things outside the protocol
can still ask for it: BlueZ before 5.67 opens EATT to connecting phones
(the server package turns that off, see linux.md, "Bookworm"), and
bluetoothd on a Linux machine pairs on its own with a peer that publishes a
GATT service requiring encryption (BLE MIDI from WirePlumber; documented,
not fixable here).

## Transports

**L2CAP connection-oriented channel** (main). The server listens on a
dynamic PSM with security level low; the kernel (Linux), CoreBluetooth
(Apple) or the Android stack handle flow control, segmentation and the end
of the link, and a channel is a plain byte stream carrying raw SSH. Several
channels can share one link, so several sessions from one client cost
nothing extra. The PSM is published in the INFO characteristic (`psm=`)
and in Apple's PSM characteristic for iOS and macOS clients.

**GATT stream** (fallback). BleStream v1: frames in characteristic writes
and notifications with its own flow control. It works on every platform,
including Web Bluetooth and Windows, at a few KB/s and one session at a
time, and it depends on a single ATT bearer (EATT off). It stays as the
lowest common denominator and for clients that already exist.

A client reads INFO after connecting; if it names a PSM and the platform
has L2CAP, the channel is used, otherwise the GATT stream. `blessh proxy
--gatt` forces the stream for tests.

Rejected alternatives and why: BR/EDR PAN and RFCOMM need classic pairing
or MFi on iOS; IPSP/6LoWPAN exists on Linux only; a custom VPN over BLE
doubles the stack. Details in [research/](research/).

## Components

```
crates/blessh-core     protocol code without I/O: BleStream frames and link state
                       machine, the byte relay for channels, INFO, the session
                       preamble; the loopback test harness
crates/blessh-ble      platform Bluetooth layer shared by server and client:
                       BlueZ (GATT packet transports, L2CAP helpers) and
                       CoreBluetooth (central, peripheral, L2CAP streams)
crates/blessh-server   the server as a library: `blessh server run` (daemon),
                       enable/disable/status, `session` (Linux root helper)
crates/blessh-client   the one program, blessh: ssh wrapper, scan, proxy, and
                       `blessh server` from blessh-server
packaging/linux        systemd units, sshd_config, D-Bus policy, sysusers,
                       maintainer scripts
packaging/macos        Info.plist embedded in the program, the sshd launchd
                       job and its configuration, tarball and signing
packaging/homebrew     the formula template for the tap
web/                   web client (planned)
```

On Linux the daemon runs as `_blessh` with a D-Bus policy that allows only
the BlueZ calls it makes, and a hardened systemd unit. It registers the
GATT service and the advertisement with bluetoothd and opens the L2CAP
listener directly in the kernel (no privilege is needed for dynamic PSMs).
Every session, whichever transport, becomes one connection to
`/run/blessh/session.sock`, a systemd socket with `Accept=yes` that starts
`blessh-session@.service` as root: `blessh server session` reads one preamble line
(session id, device address and type, transport), logs it, and execs
`sshd -i -f /etc/blessh/sshd_config`, which only includes the system
`sshd_config`. sshd sees the client as "UNKNOWN", so `Match Address` and
`PerSourcePenalties` do not apply to BLE.

On macOS the server is a Homebrew service (a LaunchAgent) in the user's
session, since CoreBluetooth refuses daemons, and serves INFO and the PSM
but not the GATT stream. It hands every channel to a launchd job that
`blessh server enable` installs once: Apple's `sshd -i` per connection on
127.0.0.1, with `/etc/blessh/sshd_config` including the system
configuration, so no blessh code runs as root
([macos-server.md](macos-server.md)). CoreBluetooth's managers run their
delegates on a private dispatch queue; a channel's NSStreams are served by
a thread of their own and appear as a tokio stream.

The client (`blessh`) finds the server through bluetoothd, connects, reads
INFO, opens the channel through the kernel (or the GATT stream through
bluetoothd) and relays stdin and stdout for ssh. On macOS it does the same
through CoreBluetooth, selecting servers by name or CoreBluetooth
identifier, since the system shows no Bluetooth addresses. `blessh user@server`
execs the system ssh with a ProxyCommand and a HostKeyAlias, so every ssh
feature works unchanged.

## Sessions

* One session per L2CAP channel; one at a time over the GATT stream.
* `--max-sessions` (default 4) bounds all sessions together; the session
  socket's `MaxConnections` must allow as many. A channel opened beyond
  the limit is refused in the channel handshake (`busy (4 sessions)`,
  which the client prints) and closed. A GATT client beyond the limit gets
  CLOSE 1.
* The server does not end a link on its own while it carries a session.
  Ten seconds after the last channel of a link closes, the server
  disconnects it unless a new channel opened; controllers that cannot
  advertise while connected become visible again that way.
* Every session has a random 64-bit id, logged by the daemon with the
  device address, the transport and, at the end, duration, byte counts and
  the reason; the session helper logs the same id before sshd's lines
  (Linux); on macOS the server logs the session's local TCP port, which
  sshd logs too.
* Phones use resolvable private addresses: the address identifies a
  connection, not a device.

## Verified so far

With two virtual controllers (btvirt) on Linux 6.17 and BlueZ 5.72: an
unprivileged L2CAP listener with a dynamic PSM; connects on a fresh link
both directly through the kernel and after bluetoothd's `Device.Connect`;
several channels on one link; link loss reported on both sides within a
second; the channel unaffected by `enable_ecred=0` (Bookworm); no loss
with a reader stalled for seconds; the full end-to-end test over both
transports (`tests/e2e-vhci.sh`). Not verifiable on btvirt and still to be
checked on real radios: two client devices connected at the same time
(the kernel re-enables advertising after a connection; the emulator does
not deliver it), phones, and Apple's CoreBluetooth against a BlueZ server.

## Known limits

* Anyone in range can attempt logins; only sshd's policy limits them, and
  not per source. Use keys, not passwords.
* A lost link ends the session; there is no resumption.
* The advertised name is public, like a host name on a network.
* The GATT stream shares BlueZ's limits: one session, EATT off on both
  sides, and before BlueZ 5.76 other devices nearby can disturb it.
