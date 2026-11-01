# blessh

[![CI](https://github.com/lomalkin/blessh/actions/workflows/blessh.yml/badge.svg)](https://github.com/lomalkin/blessh/actions/workflows/blessh.yml)
[![License: BSD-2-Clause](https://img.shields.io/badge/license-BSD--2--Clause-blue.svg)](LICENSE)

SSH over Bluetooth Low Energy: `blessh alice@my-pi` from a laptop, with no
pairing, no network and no IP address to know. One program, `blessh`, is
the client and the server, on Linux and macOS.

## Principles

* **No pairing.** The BLE link is neither paired nor encrypted; SSH does
  all of it: the client pins the host key, users log in with their keys or
  password.
* **The system's SSH server.** BLE sessions get the system sshd's
  configuration, host keys and login policy: what is allowed or refused
  over the network is allowed or refused over BLE.
* **Minimal changes to the system.** The Bluetooth stack is not
  reconfigured and other Bluetooth devices keep working; there is nothing
  to configure. On macOS no blessh code runs as root.
* **Any client, any server.** Every client works with every server on
  every platform. The [protocol](docs/protocol.md) is the specification
  they all follow, open to other implementations, and it only grows in
  ways that keep installed clients working
  ([compatibility.md](docs/compatibility.md)).

## Platforms

| Platform | Server | Client |
|---|---|---|
| Linux: Debian, Ubuntu, Raspberry Pi OS (BlueZ 5.55 or later) | yes | yes |
| macOS | yes | yes |
| Android 10 and later | — | ConnectBot BLE (coming) |
| iOS, Windows, web | — | planned |

# Installing

## macOS

```
brew install lomalkin/tap/blessh
```

To serve this Mac over Bluetooth as well (optional):

```
blessh server enable     # asks for sudo once
blessh server status
blessh server disable
```

macOS asks once whether blessh may use Bluetooth. Remote Login does not
have to be on. Details: [docs/macos-server.md](docs/macos-server.md).

## Debian, Ubuntu, Raspberry Pi OS

Download the packages for your architecture (`arm64` or `amd64`) and
`SHA256SUMS` from [Releases](https://github.com/lomalkin/blessh/releases),
then:

```
sha256sum -c --ignore-missing SHA256SUMS
sudo apt install ./blessh-client_<version>_<arch>.deb    # client
sudo apt install ./blessh-server_<version>_<arch>.deb    # server, after the client
```

The server starts right away and at every boot; the installer prints a
table of checks and the host key fingerprint. Details:
[docs/linux.md](docs/linux.md).

# Usage

```
blessh scan                          # servers in range
blessh alice@my-pi                   # connect by the advertised name
blessh -L 8080:localhost:80 my-pi    # any ssh option or command
```

`blessh` runs the system ssh, so every ssh feature works. The server's
host key is pinned as `blessh-<name>` in `~/.ssh/known_hosts`: compare its
fingerprint on the first connection. Without a user, `blessh` logs in as
the current one. The same from `ssh_config`:

```
Host my-pi
    ProxyCommand blessh proxy my-pi
    HostKeyAlias blessh-my-pi
```

More: [docs/client.md](docs/client.md).

# Settings

| What | Linux | macOS |
|---|---|---|
| Server name | `/etc/hostname`, or `--name` in `/etc/default/blessh` | the computer's name, or `/etc/blessh/name` |
| SSH settings for BLE only | above the `Include` in `/etc/blessh/sshd_config` | same |
| Server options | `BLESSH_ARGS` in `/etc/default/blessh` | — |
| Server state | `blessh server status` | `blessh server status` |
| Server log | `journalctl -u blessh-server -u 'blessh-session@*'` | `$(brew --prefix)/var/log/blessh.log` |
| Client cache | `~/.cache/blessh/` | `~/.cache/blessh/` |

# Documentation

* [docs/protocol.md](docs/protocol.md): the specification; what a
  compatible client or server must do, with test vectors. Start here to
  write one. [docs/compatibility.md](docs/compatibility.md): how it may
  change.
* [docs/linux.md](docs/linux.md): packages, server settings, logs and
  audit, requirements, desktop systems, Bookworm.
* [docs/macos-server.md](docs/macos-server.md): the macOS server, and why
  it reaches sshd the way it does.
* [docs/client.md](docs/client.md): the `blessh` command, transports,
  server names, platform notes.
* [docs/SECURITY.md](docs/SECURITY.md): recommendations, threat model,
  reporting vulnerabilities.
* [docs/architecture.md](docs/architecture.md): design, platforms and
  components of this implementation.
* [docs/development.md](docs/development.md),
  [docs/testing.md](docs/testing.md): building, testing, releasing.

# License

BSD 2-Clause, see [LICENSE](LICENSE).
