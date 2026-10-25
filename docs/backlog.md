# Backlog

Ideas worth doing, not scheduled yet.

## Access control and abuse

* **Restrict who may log in over BLE.** `AllowGroups ble-term` above the
  `Include` in `/etc/ble-term/sshd_config`; the package creates the group,
  the administrator adds users. Today BLE allows whoever network SSH
  allows.
* **Password guessing.** Anyone in radio range can connect without pairing,
  and sshd sees no source address (`UNKNOWN`), so per-source limits in sshd
  do not apply. Options: rate-limit sessions per BLE device in the daemon.
  (Public keys only is the administrator's choice in the system SSH
  configuration, which BLE follows since 0.2.0.) Caveat: phones use resolvable
  private addresses that change every few minutes, so the BLE address is a
  weak identity for rate limiting and for the audit log.
* **Slot capture.** v1 serves one client; a stranger can keep the slot busy
  (10 s handshake timeout, then up to sshd's `LoginGraceTime`, 120 s by
  default, before authentication). Options: several sessions at once (needs BlueZ ≥ 5.69,
  which keeps a socket per device), letting a new client pre-empt one that
  has not authenticated, logging repeated attempts.

## Clients

Options for clients beyond Android and Linux, cheapest first (details:
[research/client-other-platforms.md](research/client-other-platforms.md)):

* `ProxyCommand` helpers for macOS and Windows, ideally one crate with
  `ble-term-connect`.
* Android bridge app (BLE to localhost) for any Android SSH client.
* Web client (Web Bluetooth + WASM SSH): decided against for now (origin
  trust, supply chain, unverified browser write behaviour).
* iOS app.

## Packaging

* **Duplicate `/run/sshd` in tmpfiles.** The package creates `/run/sshd`
  for `sshd -i` without ssh.service. openssh-server in Debian 13 ships its
  own tmpfiles line for it, so systemd-tmpfiles warns "Duplicate line for
  path /run/sshd, ignoring". Harmless; Debian 12 needs our line.
