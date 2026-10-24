> Research working notes (2026-10-24), appendix to `../ble-terminal-research.md`. Paths like `scratchpad/repos/...` refer to temporary clones; line numbers match the SHAs given.

# C. Linux server side: BLE terminal on BlueZ (research notes)

Scope: Linux/BlueZ peripheral that exposes an interactive login terminal to native Android/iOS
clients over BLE (GATT and/or L2CAP CoC). Research only, no implementation.

Legend: **[V]** = verified in source/docs (citation given). **[H]** = hypothesis / from memory /
needs testing on hardware. Paths below are relative to
`/tmp/claude-1000/-home-user--projects-ble-term/44e98b07-3485-4b11-a71e-3f64c9e95675/scratchpad/repos/`.

Source snapshots used:

| Source | Ref |
|---|---|
| BlueZ upstream (`bluez/`) | `a0580dad5655e86314a0a777eb5f602c237a7e19` (2026-10-23, configure.ac says 5.87) |
| BlueZ tags 5.55/5.66/5.67/5.68/5.69/5.82 `src/gatt-database.c` | raw.githubusercontent.com/bluez/bluez/<tag>/..., copies in `bluez-tags/` |
| Linux kernel (`linux-bt/`) | torvalds/linux `17bb0e2f6c8043f70a693a5f3797471cbed354c8` (master, 2026-10) |
| bluer | `57ec704503417a6476bca5d3bb01122686583709` (v0.17.4) |
| python-bluezero | `9ce1bf664e14f1a174200371680a1d75e94a715c` (2025-03-23) |
| bless | `55a1549742f02d160d40a34284b524c497e02420` (2026-04-11) |
| sorah/bluetooth-getty | `c98973e92a02688c87b9819551845736bd3b37c4` |
| monperrus/asteroid-ble-shell | `36d19c6ad8fba00007d4c974f2b1017ce3965f8d` |
| radxa/linkr-bee | `8ea747350004cbd72e1b5decf3f5841b72e87b70` |
| util-linux | `fbfd2ef405d048e8fc01b309b051d2b403b09853` (login.c, agetty-cmd/*) in `../dl/` |
| systemd | `0f0682b773b15ff350057a80763cd39d71d71f30` (ptyfwd.c, pam_systemd.c) in `../dl/` |

Distribution versions **[V]** (sources.debian.org API; archive.raspberrypi.com Packages.gz, 2026-10-24):
- Debian bookworm bluez `5.66-1+deb12u2`; Raspberry Pi OS bookworm `5.66-1+rpt2+deb12u2`.
- Debian trixie bluez `5.82-1.1`; Raspberry Pi OS trixie `5.82-1.1+rpt2`. sid/forky `5.87-5`.
- polkit: bookworm 122 (JS rules), trixie 126. systemd: trixie 257, bookworm 252 (254 in bpo).
- util-linux: bookworm 2.38.1, trixie 2.41.5.
- RPi bookworm bluez patch series (`dl/rpt/debian/patches/series`) has no backport of the per-device
  Acquire* fix (see 1.4).

---------------------------------------------------------------------------------------------------

## 1. BlueZ GATT server over D-Bus

### 1.1 Registration API
- `GattManager1.RegisterApplication(object app, dict options)`; app is identified by object path +
  bus connection unique name (`bluez/doc/org.bluez.GattManager.rst`, "Methods"). The app exports an
  ObjectManager with `GattService1` / `GattCharacteristic1` / `GattDescriptor1` objects. **[V]**
- `LEAdvertisingManager1.RegisterAdvertisement(object adv, dict options)`
  (`bluez/doc/org.bluez.LEAdvertisingManager.rst:29`). Advertisement object implements
  `LEAdvertisement1` (`doc/org.bluez.LEAdvertisement.rst`). Read-only properties `ActiveInstances`,
  `SupportedInstances`, `SupportedCapabilities` (MaxAdvLen etc.) (`...LEAdvertisingManager.rst:77-130`). **[V]**
- Both registrations are bound to the D-Bus connection: if the process dies, bluetoothd drops them
  (`src/advertising.c:225 client_disconnect_cb`). Good for "removable package" behaviour: nothing is
  persisted in bluetoothd config. **[V]**

### 1.2 Who may call it (D-Bus policy)
- Upstream `bluez/src/bluetooth.conf:10-28`: `<policy user="root">` may own `org.bluez` and send the
  callback interfaces (GattCharacteristic1, LEAdvertisement1, Agent1, Profile1 ...);
  `<policy context="default"><allow send_destination="org.bluez"/></policy>` — i.e. **any local user
  may call any org.bluez method**, including RegisterApplication / RegisterAdvertisement. Upstream
  has had this since commit 3ef0ce954b66 (2018-11-06, "remove deprecated at_console statement"). **[V]**
- bluetoothd → app callbacks (WriteValue, AcquireNotify...) are allowed because bluetoothd runs as root
  and the root policy lists those `send_interface`s. So an app running as an unprivileged system user
  works without any extra D-Bus policy drop-in, as long as it does not need a well-known bus name. **[V]**
  (Only needed if we want `Type=dbus`/well-known name — then ship a small policy like
  `sorah_bluetooth-getty/contrib/dbus/jp.sorah.BluetoothGetty.conf`.)
- Debian bookworm (5.66) ships a `bluetooth.conf.patch` that additionally adds
  `<policy group="bluetooth">` allow send_destination=org.bluez; trixie 5.82 file is identical to
  upstream (fetched from sources.debian.org/data/main/b/bluez/<ver>/src/bluetooth.conf). **[V]**
- No uid/polkit check inside bluetoothd for RegisterApplication/RegisterAdvertisement (grep of
  `src/gatt-database.c`, `src/advertising.c`). **[V]**
- Security consequence **[V+H]**: any local user can register a competing GATT service or
  advertisement, call `Adapter1.RemoveDevice`, `Device1.Disconnect`, etc. The design must not assume
  bluetoothd is a privilege boundary between local users; the auth boundary is login/PAM (plus BLE
  link security).

### 1.3 Characteristic flags and link security
Flag parsing: `bluez/src/gatt-database.c:1736 parse_chrc_flags`. **[V]**
- `encrypt-write` -> PROP_WRITE + `BT_ATT_PERM_WRITE_ENCRYPT` (`:1787-1789`);
  `encrypt-authenticated-write` -> `..._WRITE_AUTHEN` (`:1793-1795`); `secure-write` -> `..._WRITE_SECURE`
  (LE Secure Connections required) (`:1799-1801`).
- `encrypt-notify` / `encrypt-authenticated-notify` / `secure-notify` set the **CCC descriptor** write
  permission and PROP_NOTIFY (`:1804-1812`) — i.e. subscribing requires that security level.
- Note: `encrypt-*-write` always adds PROP_WRITE (write-with-response); combine with
  `write-without-response` for a fast path; the attribute permission applies to both opcodes.
- `authorize` -> prepare-write authorization only (`:1802`).
- Pairing IO capability for peer-initiated pairing comes from the *default agent*; with no agent
  registered bluetoothd sets `NoInputNoOutput` (`bluez/src/adapter.c:9523-9524`) -> only Just Works ->
  the link is **unauthenticated**, so `encrypt-authenticated-*` / `secure-*` would fail. To get MITM
  protection the server must register an Agent (`doc/org.bluez.AgentManager.rst:25-48`) with
  DisplayOnly/KeyboardDisplay and become default agent (`RequestDefaultAgent`, ":82 Special permission
  might be required") — this collides with desktop agents (GNOME/KDE/bt-agent). **[V]** →
  Recommendation: require `encrypt-*` (LE SC Just Works gives passive-eavesdropping protection) and rely on
  PAM login + an application-layer authenticated channel; offer optional agent mode for headless boxes
  (passkey displayed on console/journal/e-ink). **[H, design]**
- `[General] JustWorksRepairing = never` default (5.66 and 5.82 main.conf:103) — if a phone forgets its
  bond and re-pairs, bluetoothd refuses Just Works re-pairing for an already-bonded address; the
  server must remove the stale bond (`Adapter1.RemoveDevice`) or admin intervention. **[V for default,
  H for exact UX on iOS]**

### 1.4 WriteValue / StartNotify (PropertiesChanged) path
- `WriteValue(value, options)` options include `device` (object path), `link` ("LE"/"BR/EDR"),
  `mtu`, `offset`, `prepare-authorize` — built in `append_options`
  (`bluez/src/gatt-database.c:2361-2396`, `"device"` at :2382, mtu at :2392-2395). Present in 5.66 too
  (`bluez-tags/gatt-database-5.66.c:2339`). → per-client demux of **input** is possible. **[V]**
- Notifications via `PropertiesChanged(Value)`: `property_changed_cb` (`:3045-3076`) calls
  `send_notification_to_devices` which does `queue_foreach(database->device_states, ...)`
  (`:1523-1542`) → **broadcast to every subscribed device**; no per-device targeting. **[V]**
- Value truncated to `BT_ATT_MAX_VALUE_LEN` (`:3069`) and then to MTU-3 inside
  `bt_gatt_server_send_notification` (`src/shared/gatt-server.c:1828` `length = MIN(data->len - data->offset, length)`
  where `data->len = mtu-1` and the 2-byte handle is already in the PDU) — **silent truncation**, app
  must chunk to `mtu-3`. **[V]**
- Backpressure: `bt_att_send` pushes notifications to `att->write_queue` with plain `queue_push_tail`
  (`src/shared/att.c:1681-1687`), unbounded; drained by `can_write_data` when the ATT socket is
  writable (`att.c:549-601`). PropertiesChanged signals are also queued by dbus-daemon/bluetoothd.
  → **No backpressure reaches the app; a fast producer (e.g. `cat bigfile`) grows bluetoothd memory and
  latency without bound.** The app must self-pace (application-level credit window acked by the
  client, or indications = 1 outstanding PDU, `ind_queue` per channel). **[V code; H magnitude]**
- `StartNotify`/`StopNotify` are per characteristic, counted by `chrc->ntfy_cnt` (`ccc_write_cb`
  `:2964-3043`), not per device.

### 1.5 AcquireWrite / AcquireNotify (fd fast path)
Docs: `bluez/doc/org.bluez.GattCharacteristic.rst:156-246`. Returns `(fd, uint16 mtu)`; "Only sockets
are supported"; "a HUP is generated in case the device is disconnected"; "the MTU can only be
negotiated once ... this method may be delayed in order to have the exchange MTU completed ... fd is
closed during reconnections". Server presence of `WriteAcquired`/`NotifyAcquired` properties signals
support (`:322-342`). For indications, on confirmation bluetoothd writes one byte 0x01 to the fd if
POLLOUT (`:224-227`, impl `sock_io_write` `gatt-database.c:2615-2631`). **[V]**

Implementation (upstream, >= 5.69):
- Per-client IO objects: `struct client_io` in `chrc->write_ios` / `chrc->notify_ios` keyed by
  `bt_att` (`gatt-database.c:127-128, 2748-2764, 2874-2892`). AcquireNotify is called from the CCC
  write (`ccc_write_cb` `:3016-3026`), i.e. **once per subscribing device**, with options
  device/link/mtu. **[V]**
- `sock_io_read` (`:2634-2667`) reads up to 512 B per `read()` and sends **one notification to that
  device only** (`find_device_state_by_att` + `send_notification_to_device`). With SOCK_SEQPACKET one
  app `send()` = one notification. **[V]**
- ATT disconnect → `att_disconnect_cb` → `io_shutdown(client->io)` (`:2708-2716`) → app sees HUP on
  that per-device fd = clean per-client disconnect signal. **[V]**
- Writes: `chrc_write_cb` (`:3305-3367`): if a client_io exists, `sock_io_send` (`sendmsg(...,
  MSG_NOSIGNAL)`, `:2686-2706`) and the ATT write is **acknowledged immediately** (`:3348-3355`) — app
  cannot reject individual writes. If `sendmsg` fails (EAGAIN because the app is not draining a
  non-blocking socketpair) → `BT_ATT_ERROR_UNLIKELY` for write-request, **silent drop for
  write-without-response**. If the app creates a *blocking* socketpair, bluetoothd's own send could
  block the daemon → always create with `SOCK_SEQPACKET|SOCK_NONBLOCK|SOCK_CLOEXEC` (as
  `bluez/client/gatt.c:2568` and `bluer/bluer/src/gatt/mod.rs:338-344` do). **[V code; H for blocking
  consequence]**
- Notify backpressure: `sock_io_read` keeps reading the app socket whenever readable and pushes into
  the unbounded ATT write_queue → the AF_UNIX socket buffer does **not** provide end-to-end
  backpressure either. **[V]**
- MTU: the `mtu` passed is `bt_gatt_server_get_mtu()` (ATT MTU). Payload max is mtu-3. bluer applies
  `mtu_workaround = mtu - 5` with comment "BlueZ drops data at end of packet if full reported MTU is
  used" (`bluer/bluer/src/gatt/mod.rs:358-364`). **[V]**
- `GattCharacteristic1.MTU` property exists only for remote (client-side) characteristics
  (`bluez/src/gatt-client.c:1738`); for a server the MTU arrives in each callback's options. **[V]**

**Critical version split [V]:** per-device Acquire IO was introduced by commit
`8eb1dee87e01` "gatt: Fix not establishing a socket for each device" (2023-07-24), first in **5.69**
(5.66/5.67/5.68 have 0 occurrences of `notify_ios`; 5.69+ have them). In **5.66 (Debian/RPi OS
bookworm)**:
- single `chrc->write_io` / `chrc->notify_io` per characteristic (`bluez-tags/gatt-database-5.66.c:122-123`);
- notify fd data is **broadcast to all devices** (`:2562-2580` → `send_notification_to_devices`);
- all devices' writes are merged into one fd with no device tag (`:3161-3166`);
- no ATT-disconnect → fd shutdown hook (no `att_disconnect_cb`), so HUP-on-disconnect is not
  guaranteed for server fds.
=> On bookworm, multi-client over a single characteristic pair is only safe with `WriteValue`
(device option) for input and ... no per-device output at all (PropertiesChanged is broadcast). Options
on 5.66: (a) single client at a time (reject/ignore others, `Device1.Disconnect()` extra centrals),
(b) L2CAP CoC for data (per-connection sockets, kernel-level), (c) ship a newer bluez (intrusive).
Trixie (5.82) is fine.

### 1.6 Connection / disconnection handling
- `Device1.Connected` (readonly, `doc/org.bluez.Device.rst:300`), `Paired`/`Bonded`/`Trusted`
  (`:287-307`), `Disconnect()` (`:59`) — watch via `PropertiesChanged` on `org.bluez.Device1` and
  `InterfacesRemoved`. Works on all versions; required on 5.66 since Acquire fds don't HUP. **[V]**
- Device identity: the `device` path is `/org/bluez/hciX/dev_<ADDR>`; phones use resolvable private
  addresses — only after bonding (IRK distribution) is the path stable across connections. Bind
  sessions to the D-Bus device path for the lifetime of one connection, not across reconnects unless
  bonded. **[H, standard BLE behaviour]**
- Link-loss detection latency = LE supervision timeout (connection parameters chosen by the phone
  central). **[H]**

### 1.7 Advertising and multiple simultaneous centrals
- Kernel clears `HCI_LE_ADV` when an LE connection completes (`linux-bt/hci_event.c:5893-5898`, "Advertising
  stops when a connection is created") and re-enables advertising only on **disconnect** of a
  peripheral-role link (`hci_event.c:3574-3590`, comment quoting Core spec v4.0). Ext-adv: the set is
  marked `adv->enabled = false` on `LE Advertising Set Terminated` (`hci_event.c:6154-6160`). **[V]**
- Whether a connectable advertisement may run while already in peripheral role is gated by controller
  `le_states` (`linux-bt/hci_sync.c:978-1017 is_advertising_allowed`). **[V]**
- => To accept a 2nd client the daemon must re-arm advertising after each connection (e.g.
  Unregister+RegisterAdvertisement, or a second instance) and the controller must support it
  (Raspberry Pi CYW43455/CYW43438 `le_states` and max LE links: **[H] needs on-device test**,
  `btmgmt info`/`hcitool cmd 0x08 0x001c` to read LE supported states).
- `SupportedInstances` tells how many adv sets are available. 5.66 accepts MinInterval/MaxInterval/TxPower/
  Discoverable without `--experimental` (5.66 parser table has no experimental entries even though
  5.66 `doc/advertising-api.txt` labels them [Experimental]); upstream marks only
  `ScanResponse*` parsers experimental (`bluez/src/advertising.c:1321-1345`). **[V]**

### 1.8 main.conf settings that matter (we should NOT edit main.conf)
From `bluez/src/main.conf` (5.82/upstream) and 5.66 copy: **[V defaults]**
| Setting | Default | Relevance |
|---|---|---|
| `[GATT] ExchangeMTU` | 517 | max ATT MTU bluetoothd offers; fine |
| `[GATT] Channels` | 3 in 5.66, 1 in 5.82 | EATT; with >1 notifications may use EATT bearers with own MTU **[H]** |
| `[GATT] Cache` | always | fine |
| `[GATT] KeySize` | 0 | could require 16 — global, leave alone |
| `[General] Privacy` | off | server uses public address; fine for discovery |
| `[General] JustWorksRepairing` | never | see 1.3 |
| `[General] SecureConnections` | on | LE SC available |
| `[General] Experimental` / `-E` | false | **not needed** for GATT server, Acquire*, advertising, L2CAP |
| `[LE] Min/MaxConnectionInterval` | unset | only for central-initiated; peripheral can request via L2CAP conn param update — not exposed in D-Bus **[H]** |
Nothing requires `--compat` or `--experimental`. Prior art that edits bluetoothd flags
(`VishovVladimir_ble_tty` README: "configures `bluetoothd --experimental`") is unnecessary.

---------------------------------------------------------------------------------------------------

## 2. L2CAP Connection-oriented Channels (LE CoC) on Linux

### 2.1 Socket API **[V]** (`linux-bt/l2cap_sock.c`, `bluetooth.h`, `l2cap.h`)
- `socket(AF_BLUETOOTH, SOCK_SEQPACKET|SOCK_STREAM, BTPROTO_L2CAP)`; `bind()` with
  `sockaddr_l2{l2_bdaddr=adapter or ANY, l2_bdaddr_type=BDADDR_LE_PUBLIC/RANDOM, l2_psm}`; `listen()`, `accept()`.
- Only `SOCK_RAW` needs `CAP_NET_RAW` (`l2cap_sock.c:2072`). No privilege for SEQPACKET/STREAM.
- PSM rules: LE PSM must be <= 0x00FF (`l2cap_validate_le_psm`, `l2cap_sock.c:69-80`); PSM < 0x0080
  (SIG-assigned) needs `CAP_NET_BIND_SERVICE` (`:76`). **Dynamic LE range 0x80-0xFF needs no
  capability.** BR/EDR: PSM < 0x1001 needs CAP_NET_BIND_SERVICE (`:56-67`). Constants
  `L2CAP_PSM_LE_DYN_START 0x0080`, `L2CAP_PSM_LE_DYN_END 0x00ff` (`l2cap.h:253-254`).
- `l2_psm = 0` → kernel auto-assigns first free LE PSM 0x80..0xFF (`l2cap_core.c:202-239 l2cap_add_psm`);
  EADDRINUSE if taken (`:208-210`). Only 128 dynamic LE PSMs system-wide per address.
- Mode: LE address ⇒ `L2CAP_MODE_LE_FLOWCTL` automatically unless EXT_FLOWCTL chosen
  (`l2cap_sock.c:166-171, 250-255`). `BT_MODE` sockopt (`BT_MODE_LE_FLOWCTL 0x03`, `BT_MODE_EXT_FLOWCTL 0x04`,
  `bluetooth.h:155-161`) returns ENOPROTOOPT unless `enable_ecred` (module param / `CONFIG_BT_LE_L2CAP_ECRED`,
  `l2cap_core.c:43, 8336`; `l2cap_sock.c:725-728, 1130-1133`). Use plain LE_FLOWCTL (what iOS/Android open).
- MTU: `BT_SNDMTU`/`BT_RCVMTU` (`bluetooth.h:126-127`) only for LE sockets, settable before connect
  (`l2cap_sock.c:1072-1112`). Default imtu `L2CAP_DEFAULT_MTU 672` (`l2cap.h:31`, `l2cap_sock.c:2010`);
  raise to e.g. 2048-65535 before listen. MPS derived from ACL MTU to avoid HCI fragmentation
  (`l2cap_core.c:630-631`).
- Security: `BT_SECURITY` (`bluetooth.h:65-74`): MEDIUM = encryption, HIGH = authenticated (MITM),
  FIPS = SC+MITM. Kernel refuses an incoming LE CoC connect with `L2CAP_CR_LE_ENCRYPTION` /
  `L2CAP_CR_LE_AUTHENTICATION` if the link security is insufficient (`l2cap_core.c:5271-5277`
  in `l2cap_le_connect_req`; `:5497-5502` for ECRED). Pairing itself is still done by bluetoothd/SMP
  (agent rules of 1.3 apply).

### 2.2 Flow control / backpressure **[V]**
- TX: `l2cap_chan_send` LE path segments into `chan->tx_q`, sends while credits remain, and calls
  `chan->ops->suspend()` when `tx_credits == 0` (`l2cap_core.c:2722-2749`) → `BT_SK_SUSPEND`
  (`l2cap_sock.c:1914`) → next `sendmsg` waits / EAGAIN (`bt_sock_wait_ready`, `l2cap_sock.c:1202`).
  Real end-to-end backpressure from the phone's credits.
- RX: since commit `ce60b9231b66` "Bluetooth: compute LE flow credits based on recvbuf space"
  (2024-05-14, Linux 6.10) credits granted = free socket receive-buffer space / MPS
  (`l2cap_core.c:602-622`, `l2cap_sock.c:1214-1235 l2cap_publish_rx_avail`). If the daemon stops
  reading (PTY can't take more), the phone runs out of credits → phone app's write stream blocks.
  Older kernels (e.g. 6.1/6.6) granted `(imtu/mps)+1` per SDU regardless **[H: behaviour under full
  rcvbuf on <6.10 untested]**. RPi OS bookworm currently ships 6.12 kernels **[H, check]**.
- Disconnect: socket read returns 0/error immediately on link loss/ACL disconnect — ideal session
  end trigger.

### 2.3 Coexistence with bluetoothd **[V+H]**
- LE CoC listener is a plain kernel socket; bluetoothd keeps owning the adapter via mgmt. bluetoothd
  itself listens on ATT fixed CID and (if EATT) PSM 0x27; no conflict with dynamic 0x80-0xFF.
- Still need BlueZ D-Bus for: advertising (connectable, kernel does not accept LE connections otherwise;
  bluer `examples/l2cap_server.rs:26-41` "Advertising is necessary for device to be connectable"), and a
  GATT service so the phone can discover the service and **read the PSM**.
- PSM publication: iOS convention = GATT characteristic `CBUUIDL2CAPPSMCharacteristicString`
  (`ABDD3056-28FA-441D-A470-55A75A52553A`) containing the PSM (uint16 LE) **[H: UUID from memory; client
  agent should verify Apple docs]**. Android `BluetoothDevice.createInsecureL2capChannel(psm)` /
  `createL2capChannel(psm)` needs the PSM from somewhere too → same GATT characteristic.
- Each `accept()` yields a per-client socket + `getpeername` gives peer address/type → natural
  multi-client demux independent of BlueZ version (fixes the 5.66 problem). Per-PSM security level set
  on the listening socket.

---------------------------------------------------------------------------------------------------

## 3. Libraries

| Library | Lang / license | Activity (pushed) | Stack access | GATT server | Acquire fd | per-device info | L2CAP | Verdict |
|---|---|---|---|---|---|---|---|---|
| **bluer** (bluez/bluer) | Rust, BSD-2-Clause (`bluer/Cargo.toml:16`) | 2026-08-17, v0.17.4 | BlueZ D-Bus (dbus-rs/tokio) + kernel sockets | yes (`gatt/local.rs`) | yes: `CharacteristicWriteMethod::Io`/`CharacteristicNotifyMethod::Io` → `AcquireWrite`/`AcquireNotify` (`local.rs:902-962`), SEQPACKET non-blocking socketpair (`gatt/mod.rs:338-354`) | `device_address`, `mtu`, `link` in requests (`local.rs:450-487, 738-742`) | yes: `l2cap::{StreamListener, SeqPacketListener}`, `set_security`, `send_mtu/recv_mtu/set_recv_mtu`, `set_flow_control` (BT_MODE), `conn_info` (`l2cap.rs:226-400`), PSM_LE_DYN_START (`:75`) | **Best fit** (non-exclusive, both GATT and CoC, async, static binary) |
| python-bluezero | Python, MIT | 2025-03-23 (slow) | dbus-python + GLib | yes (`localGATT.py`) | no (no Acquire* anywhere) | options dict passed to write_callback (`localGATT.py:366-374`) | no | OK for prototypes only |
| bless (kevincar) | Python, MIT | 2026-04-11 | dbus-next (D-Bus, `negotiate_unix_fd=True`, `bluezdbus/server.py:71`) | yes | AcquireNotify implemented (`bluezdbus/dbus/characteristic.py:166-198`), maps device → `central_id` | yes (read/write options["device"] → address `:131-136, 155-160`) | no | Usable; cross-platform; depends on dbus-next (unmaintained since 2024-06) |
| dbus-fast | Python, MIT | 2026-10-19 | D-Bus lib (used by bleak, HA) | DIY | fd passing supported | DIY | no | Good base for a hand-rolled Python server |
| dbus-next | Python, MIT | 2024-06-12 | D-Bus lib | DIY | yes | DIY | no | Effectively unmaintained |
| tinygo-org/bluetooth | Go (BSD-3 style; GH shows NOASSERTION) | 2026-10-17 | BlueZ D-Bus via godbus on Linux (`gatts_linux.go`) | yes, basic | no | **no** — "BlueZ doesn't seem to tell who did the write, so pass 0" (`gatts_linux.go` WriteValue) | no | Not suitable for multi-client / fast path |
| go-ble/ble | Go, BSD-3 | 2024-03-05 | raw HCI **`HCI_CHANNEL_USER`** (`linux/hci/socket/socket.go:108`), brings device down | yes | n/a | yes | own | **Rejected** (exclusive adapter) |
| @abandonware/bleno | Node, MIT | 2025-02-05 | raw HCI socket (`bindRaw`, or `bindUser` with HCI_CHANNEL_USER, `lib/hci-socket/hci.js:100-107`); README: "`bluetoothd` disabled" | yes | n/a | yes | no | **Rejected** (conflicts with bluetoothd) |
| BTstack (bluekitchen) | C, non-commercial/commercial license | 2026-10-23 | owns controller (HCI user channel / H4) | yes | n/a | yes | yes | **Rejected** (exclusive; license) |
| Adafruit Blinka bleio | Python | 2026-05-21 | bleak (central only) | **no** — `start_advertising` raises `NotImplementedError("Advertising not implemented")` (`blinka_bleio/_bleio/common.py:184-199`) | – | – | – | Not a server |
| bleak | Python, MIT | active | D-Bus | **client only** | – | – | – | Not a server |
| plain C: libdbus/sd-bus + kernel sockets | C | – | D-Bus + sockets | DIY (cf. `bluez/tools/btgatt-server.c` is raw-ATT, owns CID 4 — not for coexistence) | DIY | DIY | DIY | Smallest deps (sd-bus is in libsystemd) but most code |
"Cputil": no matching BLE library found — likely a typo; not evaluated.

---------------------------------------------------------------------------------------------------

## 4. Existing projects (prior art)

| Project | What | Server stack | Auth / security | Session handling | Assessment |
|---|---|---|---|---|---|
| **radxa/linkr-bee** (Apache-2.0) | ESP32-C3/C5 (Zephyr) dongle bridging a target's **UART console** to BLE/WiFi; clients: Web, Android/iOS (Capacitor), Rust TUI, Python/C (BlueZ D-Bus client) | Zephyr, not Linux | LE SC Just Works + bonding, physical GPIO-gated pairing, 8 bonds max (README "Pairing and security"); all chars require encryption (`docs/LINKR_BLE_API.zh-CN.md:315`) | No PTY/login: carries raw UART bytes; target's own getty does login. Only **one BLE connection at a time** (README troubleshooting) | Useful protocol lessons: "Reliable UART Service v1" — 12-byte header (magic `LR`, ver, flags, seq u32, len), payload ≤232 to fit MTU 247, writes-with-response upstream, **indications** downstream with seq + dedup + retransmit after reconnect (`src/ble_uart_reliable.h:15-18,47-50`; API doc §7 `:235-276`). `docs/BRIDGE_QUEUE_FINDINGS.md`: drop-oldest / reject-whole-write buffering causes **silent data loss** in bursts (`cat`, paste >1.6 KB). Resize: browser syncs remote tty size "after login" (they type `stty` on the UART — no out-of-band channel); native apps don't resize the remote tty (README). |
| **sorah/bluetooth-getty** (MIT, Rust) | Classic BT SPP → `/dev/rfcommN` → per-connection systemd unit with agetty | BlueZ `ProfileManager1` D-Bus | optional `--require-authentication/authorization` | Daemon `StartUnit("bluetooth-getty-session@rfcommN.service")`; unit uses `session-proxy` that **interposes a PTY** because util-linux `login` calls `vhangup()` which would kill the RFCOMM DLC (`contrib/systemd/bluetooth-getty-session@.service`, `src/session_proxy.rs:1-12`); proxy does openpty→fork→setsid→dup2→`TIOCSCTTY`→exec agetty (`session_proxy.rs:52-116`). Unit: `TTYPath=/dev/%I`, `UtmpIdentifier=%I`, `SendSIGHUP=yes`, `TTYVHangup` deliberately off. Hardened daemon unit (`CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW`, `RestrictAddressFamilies=AF_UNIX AF_BLUETOOTH`, `ProtectSystem=strict`, `SystemCallFilter=@system-service`). Ships .deb via cargo-deb with D-Bus policy + udev rule, not auto-enabled. | **Closest architectural precedent** for packaging/systemd/login; transport differs (RFCOMM, not BLE). |
| **monperrus/asteroid-ble-shell** (GPL-3.0, C++/Qt) | BLE NUS shell on AsteroidOS smartwatch (Linux+BlueZ) | BlueZ D-Bus GATT (modified asteroid-btsyncd) | RX `encrypt-authenticated-write`+`write-without-response`, TX `encrypt-authenticated-notify` (`server/src/uartservice.cpp:53,80`); requires paired+trusted peer, local "arm" window 15 min, pins single peer by `options["device"]` (`:72-76,121-143`), refuses to run as root | PTY running `/bin/sh -i` as unprivileged `ceres` (no login/PAM); `setsid` (`:214`); on disconnect/notify-stop/disarm: `kill(-pgid,SIGHUP)` → SIGTERM → SIGKILL escalation (`:271-293`). Conservative 20-byte chunks, one notification per tick (`:41-48, 383-387`); PTY fixed 80x24, no resize | Good security pattern (arm window, device pinning, auth flags, escalation kill). No PAM. |
| VishovVladimir/ble_tty (no license, Python, dbus-next) | NUS ↔ `bash -l` on RPi | D-Bus GATT, PropertiesChanged notify | **None**: root `bash -l` to anyone, no encryption flags; installer enables `bluetoothd --experimental` | `pty.openpty` + fork | Anti-pattern |
| jonasneves/pi-loop (`bridge/bridge.py`, bless) | NUS ↔ root `/bin/bash -l` on RPi image | bless | **None** ("live root shell over Bluetooth with zero config", README) | `pty.fork`, 20-byte chunks | Anti-pattern |
| hannesweisbach/ble-tty (MIT) | Creates a PTY on Linux for a remote BLE peripheral (central side) | dbus-python | – | – | Not a server |
| Jakeler/ble-serial (MIT) | Host-side (central) bridge BLE UART module → local virtual serial port | bleak | – | – | Not a server (client-side virtual tty) |
| Legacy `rfcomm watch ... agetty` | Classic SPP getty; needs `bluetoothd --compat`/sdptool | RFCOMM | – | – | Replaced by bluetooth-getty pattern |
GitHub repo/code searches ("ble getty", "nus pty", "ble shell", "ble-pty", "bluetooth terminal pty", etc.)
found no mature, maintained BLE-to-PAM-login server for Linux. **[V as of 2026-10-24 search]**

---------------------------------------------------------------------------------------------------

## 5. PTY / session lifecycle

### 5.1 Spawning
- PTY creation needs no privilege (`posix_openpt`/`openpty`; `grantpt` sets slave owner to caller). Child:
  `setsid()`, open slave (or dup2), `ioctl(0, TIOCSCTTY, 0)`, exec. Same sequence in
  `sorah_bluetooth-getty/src/session_proxy.rs:69-116`, ttyd `forkpty`+`setsid` (tsl0922/ttyd `src/pty.c:434-439`). **[V]**
- What to exec: either `agetty` on stdin port or `login` directly.
  - agetty accepts port `-` = use stdin (`util-linux agetty-cmd/agetty.c:404`), updates utmp itself
    (`agetty.c:482-484`, `utils.c:91`), may vhangup + TIOCSCTTY (`agetty-cmd/tty.c:233-285`). Provides
    `/etc/issue`, username prompt, `--login-options`.
  - `login` directly (shellinabox: `execle("/bin/login","login","-p","-h",remoteHost,...)`,
    `shellinabox/launcher.c:1685`, commit 4f0ecc31). `-h host` is root-only (`login.c:1446-1452`) and sets
    PAM_RHOST (`login.c:936`) → logind marks session remote (`pam_systemd.c:1069`). `-p` preserves env
    (TERM). PAM_TTY = tty path (`login.c:941`).
- util-linux `login` (Debian trixie+; bookworm still uses shadow's login **[H]**): closes 0/1/2,
  `vhangup()` on the tty, reopens it and **restores window size** (`login.c:600-622`); forks, parent
  stays root to run `pam_close_session` after the child exits (`login.c:1155-1230`), child `setsid()` +
  `TIOCSCTTY` (`:1243-1253`). Parent forwards SIGHUP/SIGTERM to the child's process group
  (`sig_handler`, `login.c:211-226`).
- vhangup on the slave makes the master temporarily return **EIO**; a forwarder must treat EIO on the
  master as transient while the child is alive. systemd's ptyfwd does exactly this
  (`systemd src/shared/ptyfwd.c:905-912` "EIO on the master device might be caused by vhangup()").
  bluetooth-getty documents the same issue. **[V]**
- Window size: `ioctl(master, TIOCSWINSZ, &ws)` → kernel sends SIGWINCH to the slave's foreground
  pgrp (ttyd `src/pty.c:156`). Needs an out-of-band resize message in our protocol (linkr-bee and
  asteroid lack it). **[V]**

### 5.2 Teardown on BLE disconnect
- Closing the PTY master ⇒ `pty_close()` → `tty_vhangup(tty->link)` on the slave
  (`linux-bt/pty.c:47-78`) ⇒ SIGHUP (+SIGCONT) to session leader and foreground pgrp; login forwards
  to the shell's pgrp; login then runs pam_close_session and exits. **[V]**
- Processes that ignore SIGHUP / daemonize (nohup, `tmux`/`screen` server, `setsid` children) survive —
  identical to sshd behaviour. tmux: client dies on hangup, server keeps running → user can reattach
  after reconnect (feature, not bug, if policy allows).
- **cgroup caveat [V for mechanism via pam_systemd, H for details]:** with `pam_systemd` in
  `common-session` (Debian default), logind creates `session-N.scope` under `user-UID.slice` and the
  user's processes leave our service's cgroup. So `KillMode=control-group` on our unit kills only
  agetty/login, **not** the user's shell tree. For strict "end session on disconnect":
  call `org.freedesktop.login1.Manager.GetSessionByPID(login_child_pid)` + `TerminateSession(id)` (needs
  root or polkit `org.freedesktop.login1.manage`), or `loginctl terminate-session`. logind's global
  `KillUserProcesses=` (Debian builds default "no" **[H]**) must not be changed by our package.
  Make policy configurable: `hangup` (ssh-like, default) vs `terminate` (kill everything incl. tmux).
- pam_systemd typing: tty set and not VT → `type=tty`, no seat; `remote=yes` when PAM_RHOST is a non-local
  host (`pam_systemd.c:971-1002, 1069`). Fine for pts sessions. **[V]**
- utmp/wtmp: agetty writes LOGIN_PROCESS, login writes USER_PROCESS + wtmp + lastlog
  (`login.c:640-820`); systemd `UtmpIdentifier=`/`UtmpMode=` also available in units. `who` will
  show `pts/N` with host string from `-h` (e.g. `ble:AA:BB:...`). **[V]**

### 5.3 Comparison
- sshd: does PAM itself (no login(1)), allocates pty, `PAM_TTY="ssh"` → pam_systemd special-case
  (`pam_systemd.c:971-974`). Disconnect → closes master → SIGHUP. **[V for pam_systemd part]**
- `serial-getty@.service`: `TTYPath=/dev/%I`, `BindsTo=dev-%i.device` — devpts nodes have no
  `.device` units, so it can't be reused for `pts/N` as-is; a custom template (bluetooth-getty style)
  is needed. **[H: BindsTo on pts fails/waits]**
- ttyd / gotty: run an arbitrary command in `forkpty`; "login" only if started as root with `login`
  as command; kill by `kill(-pid, sig)` (`ttyd src/pty.c:160-165`).
- shellinabox: `login -p -h` (above) or its own PAM path (`launcher.c:1089-1184`).

---------------------------------------------------------------------------------------------------

## 6. Privileges and packaging

### 6.1 What needs what **[V unless marked]**
| Operation | Privilege |
|---|---|
| RegisterApplication / RegisterAdvertisement / Device1 watch / Disconnect | any user (default D-Bus policy, 1.2) |
| AcquireWrite/Notify fds | any user (fds come from our own socketpair) |
| L2CAP LE CoC listen on PSM 0x80-0xFF | none (`l2cap_sock.c:69-80`); `RestrictAddressFamilies=AF_UNIX AF_BLUETOOTH` |
| Register default pairing Agent | any user via D-Bus, but intrusive (1.3) |
| PTY allocation | none |
| exec `login`/`agetty` (setuid to user, PAM, utmp, chown tty) | **root** (`login -h` root-only; login must run as root to switch user) |
| TerminateSession for other users' sessions | root or polkit `org.freedesktop.login1.manage` |

### 6.2 Privilege-split options
1. **Single root daemon, hardened unit** (bluetooth-getty model): simplest; `CapabilityBoundingSet` must
   keep CAP_SETUID/SETGID/CHOWN/FOWNER/DAC_OVERRIDE/AUDIT_WRITE etc. for login — so hardening is weak
   for the child; BLE parsing code runs as root (largest attack surface: untrusted radio input). Not
   recommended. **[H]**
2. **Unprivileged BLE daemon (`DynamicUser=` or `_bleterm` system user) + systemd socket-activated root
   spawner** (recommended **[H, design]**):
   - `ble-term-spawn.socket`: `ListenStream=/run/ble-term/spawn.sock`, `Accept=yes`,
     `SocketUser=_bleterm`, `SocketMode=0600`.
   - Per connection systemd starts `ble-term-spawn@.service` (root, own cgroup): allocates PTY, sends
     master fd back via `SCM_RIGHTS`, `setsid`+`TIOCSCTTY`, execs `login -p -h ble:<addr>` (or agetty `-`)
     on the slave. Unix-socket close by the daemon (BLE disconnect) ⇒ helper closes/kills and optionally
     calls logind TerminateSession. Unit stop ⇒ cgroup kill (`KillMode=control-group`).
   - No polkit rules, no D-Bus policy, privileged code tiny and never parses radio data.
   - Peer address passed as metadata must be treated as untrusted string (sanitize for `-h`).
3. **Unprivileged daemon + `StartUnit("ble-term-login@N.service")` with `TTYPath=/dev/pts/%i`**
   (bluetooth-getty-like) authorized by a polkit JS rule restricted to that unit name (polkit ≥ 0.106
   supports `action.lookup("unit")` for systemd manage-units **[H]**). Risk: a compromised daemon can
   point the root unit at *another user's* `/dev/pts/N` (tty hijack); needs an `ExecStartPre` ownership
   check. Less clean than option 2.
4. Transient units with fd passing (`StartTransientUnit` + `StandardInputFileDescriptor=`) **[H: verify
   property availability on systemd 252]** — needs polkit for transient units; more moving parts.

### 6.3 .deb shape (no edits to /etc/bluetooth/main.conf)
- `/usr/lib/systemd/system/ble-term.service` (User=_bleterm or DynamicUser, `After=bluetooth.service`,
  `Requisite`/`BindsTo=bluetooth.service`? `PartOf=bluetooth.service` so it restarts with bluetoothd
  — re-registration is required after bluetoothd restart, since registrations die with the bus
  connection) **[H]**
- `ble-term-spawn.socket` + `ble-term-spawn@.service`; sysusers.d snippet; optional
  `/etc/ble-term/config.toml`. D-Bus policy drop-in only if a well-known name is wanted.
- Removal = stop units → registrations disappear (bound to D-Bus connection); bonds created in
  `/var/lib/bluetooth/` remain (document `bluetoothctl remove`).
- Minimum versions: BlueZ ≥ 5.69 for multi-client GATT via Acquire* (else single-client or L2CAP);
  kernel ≥ 6.10 for credit-based RX backpressure on CoC (recommended, not required). systemd ≥ 247 is
  fine for everything above **[H]**.

---------------------------------------------------------------------------------------------------

## 7. Key conclusions (for the architecture doc)
1. Use **BlueZ D-Bus only** (GattManager1 + LEAdvertisingManager1) — non-exclusive, no main.conf edits,
   no `--experimental`, callable by an unprivileged user. Reject HCI-user-channel stacks (go-ble, bleno,
   BTstack).
2. GATT data path must implement **application-level flow control** — bluetoothd's notify queue is
   unbounded for both PropertiesChanged and AcquireNotify; incoming writes over AcquireWrite are
   ACKed before the app sees them and dropped on EAGAIN.
3. **BlueZ 5.66 (bookworm) cannot do per-client output on one characteristic** (Acquire fds are shared
   and broadcast; PropertiesChanged broadcasts). 5.69+ (trixie 5.82) gives per-device fds with HUP on
   disconnect. Either restrict to one client on 5.66 or use L2CAP CoC.
4. **L2CAP LE CoC is the better data channel** on Linux: no privileges (PSM 0x80-0xFF), per-client
   sockets, kernel credit flow control (true backpressure since 6.10), kernel-enforced BT_SECURITY,
   immediate disconnect signal. GATT still required for discovery + PSM characteristic (+ fallback).
5. Session: PTY + `login -p -h ble:<addr>` (or agetty `-`) under a **root spawner unit per connection**,
   driven by an unprivileged BLE daemon; forwarder tolerates EIO from vhangup; resize via TIOCSWINSZ on
   master; disconnect = close master (SIGHUP) + optional logind TerminateSession because pam_systemd
   moves user processes out of our cgroup.
6. Pairing: incoming pairing without an agent is NoInputNoOutput → Just Works only; MITM-authenticated
   flags require a default agent (intrusive). Plan Just-Works-encrypted link + PAM + app-layer
   authentication, with optional agent mode.
7. Multi-client advertising: advertising stops on connect and only auto-resumes on disconnect; the
   daemon must re-arm; controller support on Pi (CYW43455) needs testing.
