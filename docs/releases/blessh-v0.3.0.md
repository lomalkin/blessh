One Debian package, a one-command installer, and the list of servers.

**Upgrading on Debian, Ubuntu, Raspberry Pi OS:** remove the old
packages first, then install with the installer or by hand (dpkg refuses
to install `blessh` over them):

```
sudo apt purge blessh-server blessh-client
curl -fsSL https://github.com/lomalkin/blessh/releases/latest/download/install.sh | sh -s -- --server
```

* Linux: `blessh-client` and `blessh-server` are now one package,
  `blessh`, like the Homebrew formula. The server is off after
  installation; `blessh server enable` turns it on and prints its checks.
  OpenSSH's server is no longer a dependency: the server's machine needs
  `sudo apt install openssh-server`, and `enable` says so.
* `install.sh`, published with every release, installs the package for
  the machine's architecture (arm64, amd64), checked against its
  checksum; `--server` turns the server on.
* Linux server: hidden from the Bluetooth device lists of phones and
  computers, where tapping it only started a pairing; blessh clients
  find it as before. `--discoverable` in `/etc/default/blessh` shows it.
  The macOS server cannot be hidden. The protocol now asks clients to
  find servers whatever the advertising flags say; `blessh` and
  ConnectBot blessh already do.
* `blessh server status` runs the checks the Linux installer printed
  (BlueZ, EATT, adapters, sshd configuration, login policy) and shows
  them in the same table on Linux and macOS.
* `blessh` without a server lists the servers in range and connects to
  the one whose key you press; `blessh alice@` does the same as alice.
  `blessh scan` prints rows as it finds them and has `--format tsv` and
  `--format json`.
* `blessh` no longer forces `User=$USER` on ssh, so `User` in
  `~/.ssh/config` applies.
