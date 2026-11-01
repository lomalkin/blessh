# blessh server on macOS

The server on a Mac: setting it up, names, logs, and why it reaches sshd
the way it does. Installation in short: [README](../README.md#installing);
the client: [client.md](client.md); Linux: [linux.md](linux.md).

## Setting it up

```
brew install lomalkin/tap/blessh
blessh server enable     # asks for the Bluetooth permission and sudo once
blessh server status
blessh server disable    # stops it and removes the sshd job (sudo)
```

Run these as the user whose session serves Bluetooth, without sudo:
macOS grants Bluetooth to logged-in users only, so the server runs in that
user's session and is reachable only while the user is logged in. The
server uses the L2CAP channel only (the GATT stream is not available on
macOS yet; INFO says `gatt=0`, see [protocol.md](protocol.md#gatt-service)),
so it needs clients with L2CAP: Linux, macOS, Android 10 and
later, iOS.

`enable` does, leaving alone what is already in place:

1. Checks that blessh comes from Homebrew and asks macOS for the
   Bluetooth permission if it has not been decided (the service could not
   answer the prompt in the background).
2. Installs once, with sudo, the launchd job `org.blessh.sshd`: the
   system's `sshd -i -f /etc/blessh/sshd_config` for every connection to
   127.0.0.1 port 22022, and that configuration, which includes
   `/etc/ssh/sshd_config`. It creates the host keys if Remote Login was
   never on, and removes what the 0.1 tarball installed.
3. Starts the server as the Homebrew service `blessh` (`brew services
   start blessh`), which runs `blessh server run` at every login.

BLE sessions get the system's SSH settings and host keys (the same
fingerprint as Remote Login); Remote Login does not have to be on, and
nothing is opened to the network. Settings for BLE sessions only go above
the `Include` line in `/etc/blessh/sshd_config`; the file sets `SetEnv
BLESSH=1` there, so a session can tell that it came over BLE. Nothing of
blessh runs as root, so `brew upgrade` needs no sudo; a running server
restarts itself with the new version once no session is open.

`disable` stops the service and removes the launchd job; `/etc/blessh`
(the sshd configuration and the name file) stays until removed by hand.

## Names

The server advertises the computer's HostName if one is set, else its
ComputerName (System Settings, General, About, Name), read at every start.
The " (2)" that macOS appends to the ComputerName when another computer on
the network has the same name is dropped, so such renames do not change the
server's name; LocalHostName and the name DHCP hands out are not used.
`/etc/blessh/name` sets another name. Names follow host name rules
([client.md](client.md#server-names)).

## Logs

The server's log is `$(brew --prefix)/var/log/blessh.log`: every session
with its id, the client's CoreBluetooth identifier and the local port of
its connection to sshd. sshd logs the same port as the client's (`from
127.0.0.1 port N`); reading sshd's lines needs an admin:

```
sudo log show --last 1h --info --predicate 'process BEGINSWITH "sshd" AND eventMessage CONTAINS "127.0.0.1"'
```

macOS's process names (`sshd`, `sshd-session`) and the PAM service
(`sshd`) cannot be changed without replacing system files, and BSM audit
is deprecated on macOS and off by default, so the port is what ties the
two logs together. Other changes to the system's logging, such as
`LogLevel VERBOSE` for BLE sessions, are in the backlog: for now the
criterion is as little change to the system and as few settings as
possible.

## Why a localhost sshd job

The macOS server hands every BLE session to the system's sshd through a
private launchd job on localhost (option 4 below). This section records
why, and the alternatives that were compared. Linux is not affected: there the
session socket of systemd runs `sshd -i` as root with the system
configuration.

### Constraints

* **Bluetooth only in the user's session.** CoreBluetooth serves
  logged-in users; the BLE server (`blessh server run`) is a LaunchAgent
  started by `brew services`, running as that user.
* **Homebrew never runs as root.** A formula installs into a prefix the
  user owns (`/opt/homebrew`) and cannot install a LaunchDaemon. Anything
  that runs as root needs a separate step with `sudo` or an admin prompt.
* **Code that root runs must not be writable by the user.** A LaunchDaemon
  that executes a binary from the Homebrew prefix gives root to every
  process of that user. This rules out a root helper of our own (the
  `org.blessh.session` daemon of the 0.1 tarball) once blessh comes from
  Homebrew.
* **sshd on macOS needs a TCP client.** Its BSM audit session needs a
  client address; over a pipe or a Unix socket the session fails
  ("Could not create new audit session").
* **The system host keys are readable by root only.** An sshd that is not
  root cannot use them.
* **Remote Login is a launchd socket.** `/System/Library/LaunchDaemons/
  ssh.plist` (protected by SIP) listens on port 22 on every interface,
  announces `ssh` and `sftp-ssh` over Bonjour, and starts one `sshd -i`
  per connection (at most 42). `ListenAddress` has no effect, so Remote
  Login cannot be limited to localhost. As there is no listening sshd,
  `PerSourcePenalties` and `MaxStartups` do not apply.

### Options

1. **sshd as the user.** The server starts `/usr/sbin/sshd -i` itself,
   as the user, with its own host key (in
   `~/Library/Application Support/blessh`), over a loopback TCP pair.
2. **Remote Login on localhost.** The user turns on Remote Login in
   System Settings; the server connects to `127.0.0.1:22` for every
   session.
4. **Private localhost sshd (chosen).** `blessh server enable` installs
   once, with `sudo`, a LaunchDaemon like Apple's `ssh.plist` that listens
   on `127.0.0.1` only and starts the system `/usr/sbin/sshd -i -f
   /etc/blessh/sshd_config` per connection; that file includes
   `/etc/ssh/sshd_config`. The server connects to that port. Only Apple's
   sshd runs as root; the plist and the configuration are root-owned
   files that upgrades never touch.

| | 1 sshd as the user | 2 Remote Login | 4 private localhost sshd |
|---|---|---|---|
| Privileges to enable | none | an admin turns on Remote Login once | `sudo` once |
| Privileges to upgrade | none | none | none |
| Privileges to disable | none | none | `sudo` |
| Works with Remote Login off | yes | no | yes |
| SSH reachable from the network | no | yes, every interface, announced over Bonjour | no |
| Host key | its own | the system's | the system's |
| Who can log in | only the user who runs the server | every account the system allows | every account the system allows |
| Passwords, PAM | no, keys only | as the system | as the system |
| System sshd configuration | partly: included, but what needs root does not work | fully | fully, plus BLE-only settings above the `Include` |
| blessh code running as root | none | none | none |
| Verified | key login and PTY on a Mac, with its own configuration | not with blessh | over the port on a Mac (below) |

### Trade-off

Without any privilege (option 1) the server loses more than the host
key, which matters least (the client pins the key per BLE server name
either way): only one account can log in, only with keys, and the
administrator's configuration applies only where it does not need root.
Option 2 keeps the system's SSH completely but opens SSH to every network
the Mac joins and depends on a setting the user, MDM or "Allow access
for" lists can change. Option 4 keeps the system's SSH completely, stays
off the network and works with Remote Login off, for one `sudo` when the
server is enabled.

### Fragility

**1. sshd as the user.** Few parts and no privileges, but it departs from
the system's policy, as above.

**2. Remote Login.** The mechanism is robust (Apple's launchd socket, a
real TCP client, one connection per session); what is fragile is around
it:

* SSH is open to every network the Mac joins and announced with Bonjour;
  it cannot be limited to localhost.
* It can be turned off at any time; the server sees refused connections
  and must say why. Turning it on needs an admin account; `systemsetup
  -setremotelogin` also needs Full Disk Access on current macOS, so the
  server can only point to System Settings.
* An administrator's `Match Address 127.0.0.1` applies to BLE.

**4. Private localhost sshd.** The mechanism of option 2 on a port of its
own. Costs: one `sudo` at enable and at disable; a fixed loopback port
that another program could hold (enable checks it); if Remote Login was
never on, the host keys may not exist yet and enable creates them
(`ssh-keygen -A`).

Checked on a Mac with macOS 15.7 (OpenSSH 9.9) with a LaunchDaemon
listening on `127.0.0.1:22022` (`SockNodeName`, `inetdCompatibility`
without `Wait`) that runs `/usr/sbin/sshd -i -f /etc/blessh/sshd_config`:

* launchd listens on 127.0.0.1 only; the port is closed from the network.
* sshd presents the system host key (same fingerprint as Remote Login).
* Key login, a command and a PTY work; root without a key is refused.
* Six sessions in parallel and twenty in a row pass; a session takes
  about 0.23 s (0.15 s over Remote Login).
* No BSM audit errors: the connection is real TCP from 127.0.0.1.
* Twelve failed logins in a row do not block the next good one.
* Once, twenty sessions in a row took about 9 s each; three repeats of
  the same run took 0.23 s each. Not explained; to watch in the cycle
  tests.

With blessh 0.2.0 on the same Mac: BLE sessions from Linux clients
through the job, `enable` run again without asking for sudo, the 0.1
setup replaced, and a reinstall of the formula after which the server
restarted itself once idle. Not checked yet: a reboot, Remote Login off.

### Why TCP

A Unix socket instead of the TCP port is not possible (BSM audit, see
the constraints). Listening on 127.0.0.1 is enough: other local users
reach the same sshd with the same checks as `ssh localhost`.
