# Backlog

Ideas worth doing, not scheduled yet.

## Access control and abuse

* **Restrict who may log in over BLE.** `AllowGroups ble-term` in
  `/etc/ble-term/sshd_config`; the package creates the group, the
  administrator adds users. Today every local user except root may log in.
* **Password guessing.** Anyone in radio range can connect without pairing,
  and sshd sees no source address (`UNKNOWN`), so per-source limits in sshd
  do not apply. Options: rate-limit sessions per BLE device in the daemon,
  or allow public keys only by default. Caveat: phones use resolvable
  private addresses that change every few minutes, so the BLE address is a
  weak identity for rate limiting and for the audit log.
* **Slot capture.** v1 serves one client; a stranger can keep the slot busy
  (20 s handshake timeout, then up to sshd's `LoginGraceTime` before
  authentication). Options: a shorter pre-authentication limit, letting a
  new client pre-empt one that has not authenticated, logging repeated
  attempts.
