# Security

Threat model, security controls and the results of the security review of
the ble-term server. Findings are numbered (S-n) and keep their number when
their status changes.

## Reporting a vulnerability

Do not report vulnerabilities in public issues. Use GitHub's private
vulnerability reporting (Security tab, "Report a vulnerability") or a draft
security advisory; while the repository is private, contact the
maintainers directly. Include the affected version (`dpkg -s ble-term`),
the BlueZ and OS versions, and steps to reproduce. Fixes go into the next
release; only the latest release is supported. Vulnerabilities of the
Android client belong to the client's repository.

## Threat model

The server advertises a GATT service without pairing. Anyone within radio
range (tens of metres, more with a directional antenna) can connect and send
arbitrary data before authenticating. Authentication and encryption are left
to SSH: the client pins the BLE host key, users log in with a password (PAM)
or a public key.

Assets: the server (code execution, privilege escalation), user accounts
(passwords), the confidentiality and integrity of sessions, the
availability of the service, the integrity of the audit log, the rest of
the system's Bluetooth.

Attackers considered:

* **A1, nearby unauthenticated BLE device**: the main one.
* **A2, local unprivileged user** on the server.
* **A3, compromised daemon** (`_bleterm`), to bound the damage of a bug in
  our code.

Out of scope: physical access, a compromised kernel, BlueZ or OpenSSH, and
the phone.

## Attack surface and controls

| Component | Runs as | Input from A1 | Controls |
|---|---|---|---|
| bluetoothd (BlueZ) | root | ATT/GATT, SMP, L2CAP | distribution package; not modified by ble-term |
| `ble-term-daemon` | `_bleterm`, no capabilities, `NoNewPrivs`, seccomp `@system-service`, read-only file system, private `/tmp`, `AF_UNIX`/`AF_BLUETOOTH` only (systemd exposure 1.7); D-Bus policy limits it to the BlueZ calls it needs | BleStream frames | sans-IO protocol state machine with bounded buffers (reorder window 64 frames, credit-limited receive queue, 8 KiB read-ahead); fuzzed, model checked |
| `/run/ble-term/session.sock` | socket `0600 _bleterm` | none (daemon only) | systemd socket activation, `MaxConnections=4` |
| `ble-term-session` | root, per session | none directly; one line from the daemon | strict parser (fixed format, 128 bytes, 10 s timeout); execs sshd |
| `sshd -i -f /etc/ble-term/sshd_config` | root, then the user | SSH protocol | OpenSSH; no root login, `MaxAuthTries 3`, `LoginGraceTime 60`, no forwarding of any kind, no compression |

The BLE SSH host key is `0600 root`; the daemon cannot read it.

## Findings

| ID | Severity | Status | Summary |
|---|---|---|---|
| S-1 | Medium | Fixed (0.1.3) | Journal entries could be forged through CLOSE messages |
| S-2 | Medium | Documented, installer warns (BlueZ < 5.76) | Other BLE devices can inject into or observe the active session |
| S-3 | Medium | Mitigated (0.1.3), rest in backlog | One client can occupy the only session slot |
| S-4 | Medium | Mitigated (0.1.3), rest in backlog | Password guessing over BLE is not rate limited by source |
| S-5 | Low | Fixed (0.1.3) | A compromised daemon controls all of BlueZ over D-Bus |
| S-6 | Low | Fixed (0.1.3) | Unix socket forwarding was allowed |
| S-7 | Info | Documented, installer hint (0.1.3) | The host name is broadcast |
| S-8 | Info | Accepted | BLE addresses in the audit log are weak identities |
| S-9 | Info | Accepted | The daemon is trusted for the audit fields it passes on |
| S-10 | Info | Fixed (0.1.3) | Integer overflow in `ble-term-session --timeout` |

### S-1 Forged journal entries through CLOSE messages

The CLOSE frame carries a free-text message from the peer. The daemon (and
the desktop client) logged it verbatim. Under systemd the daemon writes to
the journal through a stream, where a newline starts a new entry and a
`<N>` prefix sets its priority; an unauthenticated peer could therefore add
arbitrary entries attributed to `ble-term-daemon`, including fake audit
lines. Fixed: peer text is escaped (`protocol::printable`) and truncated to
200 characters before logging.

### S-2 Shared characteristic sockets in BlueZ < 5.69 / < 5.76

In BlueZ before 5.69 the socket handed out by `AcquireWrite` and
`AcquireNotify` belongs to the characteristic, not to a device
(`src/gatt-database.c`: writes from any device go to `chrc->write_io`;
fixed by "gatt: Fix not establishing a socket for each device", 5.69).
Until 5.76 notifications sent on that socket also went to every subscribed
device ("gatt-database: Fix sending notification to all devices").

While a session runs on such a server, a second device can:

* write to RX: its packets reach the session's stream. The BleStream
  sequence check or, past it, the SSH MAC rejects them, so the effect is a
  dropped session (denial of service), not command injection;
* enable notifications on TX: it then receives the session's notifications,
  i.e. the encrypted SSH stream, frame sizes and timing. SSH confidentiality
  holds, but the timing of keystrokes is exposed, and the ConnectBot BLE
  client disables OpenSSH-style keystroke timing obfuscation to save
  bandwidth.

The busy check of the daemon does not see such a device, because BlueZ
does not call Acquire* for it.

Affected: Bookworm (5.66). Not affected: Trixie (5.82). No CVE was found
for the upstream fixes, which went in as ordinary bug fixes; Debian
bookworm's bluez 5.66-1+deb12u2 does not carry them.
Decision: BlueZ ≥ 5.76 is recommended (README) and the installer warns on
older versions. Not done: disconnecting other LE devices during a session
(it would also cut unrelated devices) and keystroke timing obfuscation in
the client (it costs bandwidth on a slow link); the exposure is accepted
with this documentation.

### S-3 Session slot occupation

v1 serves one client. A nearby device can keep the slot: up to 10 s for
the BleStream handshake (5 s if it opens only one characteristic), then up
to `LoginGraceTime` (60 s) before authenticating, and repeat. 0.1.3 halves
both limits (they were 20 s and 120 s); several sessions at once and other
options are in backlog.md.

### S-4 Password guessing

sshd sees no source address for BLE sessions, so per-source penalties do
not apply. `MaxAuthTries 3` per connection (4 before 0.1.3), the session
slot (one connection at a time, `LoginGraceTime` 60 s) and PAM (e.g.
`pam_faillock`, if configured) are the limits. BLE addresses of phones
rotate, so a per-address limit in the daemon is weak. The README
recommends public keys only and `AllowGroups`, and `sshd_config` explains
how; a default group and a session rate limit are in backlog.md.

### S-5 D-Bus access of the daemon

The daemon talks to BlueZ as `_bleterm` (group `bluetooth`). BlueZ's D-Bus
policy lets it call every BlueZ method: pair and connect devices, change
adapter settings. On Raspberry Pi OS the default policy even grants this to
every local user (Debian's BlueZ package does the same). The most serious
use for an attacker who has taken over the daemon (A3) would be to pair
and trust their own BLE keyboard and type on the server.

Fixed: `/usr/share/dbus-1/system.d/ble-term.conf` denies `_bleterm` every
call to BlueZ except the ones the daemon makes (`GetManagedObjects`,
`Properties.Get`, registering and unregistering the GATT application and
the advertisement, `Device1.Disconnect`); rules for a user override the
default and group rules of BlueZ's policy. The list was taken from a D-Bus
capture of the end-to-end test; `tests/e2e-vhci.sh` with `DAEMON_USER`
runs the daemon under the policy and checks that other calls are denied.

### S-6 Unix socket forwarding

`AllowStreamLocalForwarding` defaulted to yes. Only an authenticated user
could use it, with their own rights, so no privilege was gained; it was
still contrary to the intent of the configuration. Fixed: `DisableForwarding
yes`.

### S-7 Host name broadcast

The advertisement and the INFO characteristic contain the host name, which
anyone nearby can read. Use `--name` in `/etc/default/ble-term` to
advertise something else; the installer says so on first installation.

### S-8 Weak identities in the audit log

Phones use resolvable private addresses that change every few minutes; the
logged address identifies a connection, not a phone. The SSH login (user,
key fingerprint) logged by sshd in the same unit is the reliable part.
Accepted: identity rests on SSH (the pinned host key, whose fingerprint
the user compares on first connect, and user authentication). Pairing with
identity resolving keys would give stable phone identities and is left for
a later protocol version.

### S-9 Audit fields from the daemon

`ble-term-session` (root) logs the session id and device address the daemon
sends. Only `_bleterm` can connect to the session socket, so the line is as
trustworthy as the daemon; a compromised daemon (A3) could forge it, but
gains nothing else: sshd still authenticates.

### S-10 Timeout overflow

`--timeout` × 1000 could overflow (the value comes from the unit file, not
from the radio). Fixed with saturating arithmetic.

## Verification

| Check | Scope | Result |
|---|---|---|
| Code review | daemon, bridge, link, protocol, session wrapper, units, `sshd_config` | findings above |
| Fuzzing (`tests/fuzz`) | frame decoding, link state machine, link pairs, bridge, preamble | no crashes in our code |
| AddressSanitizer, LeakSanitizer | unit tests; loopback end-to-end test (300 KB each way, reordering, session socket) | clean |
| Miri | library unit tests without BlueZ (`cargo miri test --lib --no-default-features`) | all 13 tests clean (`sequence_wraps`, 70 000 frames, takes about 40 minutes under Miri) |
| Kani | frame decoding (`link::proofs::decode_never_panics`): every packet of up to 16 bytes | no panic (33 s). Harnesses for the link state machine were replaced by exhaustive unit tests over boundary values (frame size, window, data length; 822 edge packets alone and in pairs): CBMC ran out of memory on std's containers |
| D-Bus policy | `tests/e2e-vhci.sh` with the daemon as a separate user under the policy; calls outside the list (`StartDiscovery`, setting `Pairable`, `RemoveDevice`) | passes; other calls denied, allowed without the policy. On the Trixie and Bookworm benches with a phone: sessions, reconnects and idle-link disconnects work, and the bus logged no rejected call from the daemon |
| Bench, over the radio | two Linux machines (BlueZ 5.82 and 5.66), both directions: sessions back to back, transfers with checksums, both RX write types, link loss | passes; found and fixed the reuse of sockets of a dead link (0.1.3) |
| `cargo audit` | main crate and fuzz crate lock files | no advisories |
| `cargo deny` | licences, advisories, sources, yanked crates (`deny.toml`) | ok; one duplicate crate version warning |
| Sandbox of the running daemon | target bench (Trixie) | as in the table above |
