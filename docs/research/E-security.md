> Рабочие заметки исследования (2026-10-24), приложение к `../ble-terminal-research.md`. Пути вида `scratchpad/repos/...` относятся к временным клонам; номера строк соответствуют указанным SHA.

# E. Security: BLE terminal (headless BlueZ peripheral, Android/iOS clients, agetty/login/PAM)

Status labels: **[V]** verified by reading source or a primary or near-primary page (citation given). **[H]** hypothesis or recollection that was not re-verified in this session.

Sources pinned in this session:
- BlueZ `bluez/bluez` @ `a0580dad5655e86314a0a777eb5f602c237a7e19` (depth-1 clone, `scratchpad/repos/bluez`)
- Linux `torvalds/linux` master @ `17bb0e2f6c8043f70a693a5f3797471cbed354c8` (single files fetched to `scratchpad/repos/linux`)
- OpenSSH portable master @ `26b1363c7cbf5d01c730f45fc0b4abc5686beaf7` (single files, `scratchpad/repos/openssh`)
- ConnectBot sshlib `connectbot/sshlib` @ `9b80e3da13147723d72e411f72dc2e9237fc2884` (this is the new Kotlin "cbssh" library, v0.6.x)
- Apple `swift-nio-ssh` @ `149f32ffacda5770e513cb41bd175e3d6ef77ce3`

---

## 1. Pairing and bonding on a headless BlueZ peripheral

### 1.1 How the pairing method is chosen (kernel SMP)
- **[V]** The local IO capability comes from the **default agent's** capability and is set **per adapter** through mgmt `SET_IO_CAPABILITY` (`bluez/src/adapter.c:9508-9535`, applied at `adapter.c:9755-9760`). With no agent and `AlwaysPairable=true`, BlueZ falls back to NoInputNoOutput (`adapter.c:9523-9524`). With no agent and `AlwaysPairable=false` (the default), BlueZ turns **Bondable off** (`adapter.c:9513-9518`).
- **[V]** The LE SC method table is `sc_method[remote_io][local_io]` (`linux/net/bluetooth/smp.c:808-815`). Phones advertise KeyboardDisplay (index 4):
  - local **DisplayOnly** gives `DSP_PASSKEY`: the server displays and the phone types it.
  - local **KeyboardOnly** gives `REQ_PASSKEY`: the server must *enter* a passkey that the phone displays.
  - local **NoInputNoOutput** gives `JUST_WORKS`.
  - local **DisplayYesNo** or **KeyboardDisplay** gives `CFM_PASSKEY` (Numeric Comparison).
- **[V] A fixed or static passkey is NOT possible on Linux in the display role.** For LE SC `DSP_PASSKEY`, the kernel itself generates the passkey: `get_random_bytes(&hcon->passkey_notify, ...); passkey_notify %= 1000000` (`smp.c:2805-2815`). It then only *notifies* userspace through `mgmt_user_passkey_notify` and BlueZ `Agent1.DisplayPasskey`. Legacy pairing is the same (`smp.c:905-912`). `Agent1.DisplayPasskey` returns void (`bluez/doc/org.bluez.Agent.rst:83-99`), so an agent cannot inject a value. mgmt `USER_PASSKEY_REPLY` is only used in the `REQ_PASSKEY` (input) direction (`smp.c:1560-1584`, `1633-1640`).
  - The only way to make the server side "choose" a passkey is to declare **KeyboardOnly** and answer `RequestPasskey` with a constant (`Agent.rst:70-81`). With a phone (KeyboardDisplay) as initiator, however, the **phone** displays a random passkey that the server must type (`REQ_PASSKEY`), so a constant answer fails. **[V]** from the table. Static-passkey pairing therefore works only if *both* sides are KeyboardOnly, which phones are not. Contrast this with Zephyr `CONFIG_BT_FIXED_PASSKEY` and ESP-IDF `ESP_BLE_SM_SET_STATIC_PASSKEY` **[H, recollection]**, which can do it because they own the SMP implementation.
  - The kernel would need to be patched (for example, a debugfs or mgmt knob for a fixed TK) for a static passkey. Not recommended (see 1.4).
- **[V] Linux always asks userspace to confirm SC Just Works**, even on the responder side, "to prevent impersonation attacks" (`smp.c:2181-2192`, `confirm_hint = 1`). In BlueZ this goes to `device_confirm_passkey` (`bluez/src/device.c:7934`):
  - If the device is already paired, the **JustWorksRepairing** policy decides: `never` means reject (default), `always` means accept (`device.c:7940-7952`).
  - Otherwise, if BlueZ did not initiate the bonding itself, it calls **`Agent1.RequestAuthorization`** (`device.c:7972`; doc `Agent.rst:118-129`).
  - With no agent, `new_auth` fails (`device.c:7858-7880`) and the pairing is rejected (`adapter.c:8463-8469`).
  - **Design consequence:** a custom NoInputNoOutput agent can implement a *pairing window* or *physical-button gate* in `RequestAuthorization`, for example accepting only within 60 s of a GPIO button press. This works even though Just Works itself is unauthenticated.

### 1.2 Agent coexistence
- **[V]** `AgentManager1.RegisterAgent`: "Every application can register its own agent and for all actions triggered by that application its agent is used" (`doc/org.bluez.AgentManager.rst:30-34`). One agent per application (`:41-42`).
- **[V]** For **remote-initiated** pairing (the phone initiates; always our case), BlueZ uses `agent_get(NULL)`, which is the **head of the default-agent queue** (`agent.c:247-260`, `device.c:7874-7877`). A per-application agent therefore does not help. Our daemon must be the **default** agent (`RequestDefaultAgent`, `AgentManager.rst:76-86`), and that agent's capability sets the adapter IO capability.
- **[V]** Default agents are kept in a queue. The newest `RequestDefaultAgent` wins, and on unregister the previous one is restored (`agent.c:139-170`).
- **Implication:** on a headless Pi, conflicts arise only if someone runs `bluetoothctl` (whose `agent on`/`default-agent` would preempt) or a desktop agent (blueman, gnome-bluetooth). The service must re-assert the default agent, or document the conflict.
- **[H]** No D-Bus policy prevents another root process from becoming the default agent; root on the box is fully trusted anyway.
- Pairable, Discoverable, Bondable and IO capability are **adapter-wide**. Toggling them from the service affects every other Bluetooth use on the host (keyboards, audio). This is intrusive on a multi-purpose machine and fine on a dedicated appliance. Put this behind a config option.

### 1.3 main.conf options (`bluez/src/main.conf`)
- **[V]** `AlwaysPairable` (`:16-19`, parsed in `main.c:1083`): allows pairing without an agent. Do not use. We want an agent as the gate.
- **[V]** `PairableTimeout` (`:21-24`).
- **[V]** `Privacy = off|network/on|device|limited-*` (`:72-98`) controls local RPA use for our advertising.
- **[V]** `JustWorksRepairing = never|confirm|always`, default `never` (`:100-103`; enforcement at `device.c:7940-7952`). Keep `never` or `confirm`. `always` re-opens unauthenticated bond replacement, which is the BLERP-class problem.
- **[V]** `SecureConnections = off|on|only`, default `on` (`:113-123`; `main.c:1058-1072`; pushed with `MGMT_OP_SET_SECURE_CONN` at `adapter.c:10768`). With `only`, the kernel sets `HCI_SC_ONLY`:
  - SMP rejects pairing requests or responses without the SC bit (`smp.c:1739`, `1929`, `2301`).
  - It refuses legacy fallback ("refusing legacy fallback in SC-only mode", `smp.c:2036-2039`).
  - Equivalent at runtime: `btmgmt sc only`.
  - All iOS (>= iOS 8 era) and Android (>= 6 era) phones support LE SC **[H, widely known]**.

### 1.4 Static or reused passkey: why it is weak
- **[V/known]** In LE SC Passkey Entry the passkey is authenticated one bit per round over 20 rounds. `sc_passkey_round`, `r = (passkey >> round) & 1` (`smp.c:1453-1520`). After a run, each round's nonce and commitment let an observer or active attacker test each bit independently. A **reused** passkey is therefore recoverable from one observed or attempted pairing, and the protocol only protects a **fresh** passkey. The Bluetooth Core spec says the passkey should be random per pairing **[H: exact spec section not re-checked]**. The classic analysis is Lindell, "Attacks on the Pairing Protocol of Bluetooth v2.1", Black Hat USA 2008 **[H: citation from memory]**.
- **[V]** CVE-2020-26558 (ANSSI): Passkey Entry in SSP, SC and LE SC is vulnerable to impersonation by reflection. The attacker learns the passkey bit by bit and completes authenticated pairing. Specs 2.1 to 5.2 are affected. https://access.redhat.com/security/cve/CVE-2020-26558 , https://www.microchip.com/en-us/products/wireless-connectivity/features/security/software-vulnerability-response/anssi-bluetooth-core-vulnerabilities . BlueZ and kernel mitigations reject a remote public key equal to the local one **[H: patch detail not checked in this tree]**.
- **[V]** CVE-2020-10134 "Method Confusion" (TU Munich; Tschirschnitz et al., IEEE S&P 2021): a MITM makes one side run Numeric Comparison and the other Passkey Entry, and the user types the displayed comparison value. https://www.chromium.org/chromium-os/bluetooth-pairing-method-confusion-on-chrome-os . A *DisplayOnly* server (where the phone has to type) is exactly the setup in which the user must be told "type the code shown on the server; never 'confirm' a number".
- Conclusion: a static passkey printed on a sticker equals a shared secret that leaks after the first pairing an attacker observes or attacks. It is only marginally better than Just Works plus a pairing window. It is also not implementable on Linux without kernel changes (1.1).

### 1.5 Other relevant attacks
- **[V]** BLESA (Wu et al., USENIX WOOT 2020; CVE-2020-9770 for iOS, fixed in 13.4). This is reconnection spoofing: an attacker impersonates a previously bonded **server** to a client, which accepts unauthenticated data. BlueZ, Android and iOS were reported affected. https://www.cs.purdue.edu/homes/dxu/pubs/WOOT20.pdf , https://threatpost.com/bluetooth-spoofing-bug-iot-devices/159291 . **Relevance:** a rogue "Pi" could serve a fake login prompt to a phone and phish the password. Client apps must refuse to send anything until the link is encrypted with the bonded key, or must use app-layer server authentication (SSH host key or Noise static key).
- **[V]** BLURtooth CVE-2020-15802 (CTKD, specs 4.2 to 5.0): a bond made on one transport (LE or BR/EDR) can overwrite the other. The SIG mitigation is to be pairable only on user action. https://thehackernews.com/2020/09/new-bluetooth-vulnerability.html . **Relevance:** the Pi controller is dual-mode. Keep BR/EDR off if unused (`ControllerMode = le` in main.conf **[H: option exists in BlueZ main.conf; not re-checked in this tree]**) and keep pairable windows short.
- **[V]** BLUFFS CVE-2023-24023 affects **BR/EDR** session-key derivation (4.2 to 5.4). It is not LE, so it only matters if BR/EDR is left enabled. https://thehackernews.com/2023/12/new-bluffs-bluetooth-attack-expose.html
- **[V]** BLERP (Sacchetti and Antonioli, EURECOM, **NDSS 2026**): six re-pairing design flaws in spec v6.1 (unauthenticated re-pairing, security-level downgrade), with impersonation and MITM against 22 targets including the Apple, Android and NimBLE stacks. https://www.ndss-symposium.org/ndss-paper/blerp-ble-re-pairing-attacks-and-defenses/ . Mitigations are `JustWorksRepairing=never` (BlueZ default) and attribute permissions requiring authenticated or "secure" levels so that a downgrade fails.
- **[H, recollection]** KNOB on BLE (Antonioli, Tippenhauer and Rasmussen, ACM TOPS 2020): LE key-size negotiation down to 7 bytes. **[V]** The mitigation exists in the stack:
  - `BT_SECURITY_FIPS` requires `max_key_size == 16` (`smp.c:712-714`).
  - The BlueZ `secure-*` flags check `min_enc_size` (`bluez/src/shared/gatt-server.c:438-444`).
- **[H, recollection]** LE Legacy Just Works and Passkey can be cracked offline from a sniffed pairing ("crackle", Mike Ryan 2013, https://github.com/mikeryan/crackle). `SecureConnections=only` eliminates this.
- **[V]** BLE link-layer relay (NCC Group, Sultan Qasim Khan, May 2022): it adds about 8 ms, works through LL encryption and defeats latency bounding. https://www.nccgroup.com/research/technical-advisory-ble-proximity-authentication-vulnerable-to-relay-attacks/ . **Relevance:** proximity is *not* an authentication factor. A bonded phone in a pocket can be relayed to the Pi. Never auto-login on "bonded device in range"; always require the PAM password or key.

---

## 2. GATT attribute permissions and L2CAP security levels

### 2.1 BlueZ GATT flags (`doc/org.bluez.GattCharacteristic.rst:373-385`, parsing in `src/gatt-database.c:1784-1821`)

| Flag | ATT perm | Required link security (`shared/gatt-server.c:412-460`) | What satisfies it |
|---|---|---|---|
| `encrypt-read/write` | `*_ENCRYPT` | >= MEDIUM, else `Insufficient Encryption` | any pairing incl. Just Works |
| `encrypt-authenticated-*` | `*_AUTHEN` | >= HIGH, else `Insufficient Authentication` | MITM pairing (Passkey/NumComp; legacy passkey also gives HIGH) |
| `secure-*` (server only) | `*_SECURE` | >= FIPS **and** key size check, else `Insufficient Authentication` | **LE SC + MITM** (`authreq_to_seclevel`, `smp.c:602-612`; `pending_sec_level = FIPS` for non-JW SC, `smp.c:2797-2800`) |
| `encrypt-notify` / `-indicate`, `encrypt-authenticated-notify`, `secure-notify` | applied to the **CCC descriptor write** (`gatt-database.c:1804-1821`) | same | subscribing requires the level |

- **[V]** Practical consequence: with a **NoInputNoOutput** server, `encrypt-authenticated-*` and `secure-*` can **never** be satisfied, because Just Works only yields MEDIUM (`smp.c:2797-2798`). The phone would loop or fail. MITM-level flags require the DisplayOnly agent, which means a passkey must be shown somewhere.
- **[V]** When the client receives these errors, BlueZ's own client side escalates security (`bluez/src/shared/att.c:770-790`). Phones do the same:
  - **[H, well-known platform behaviour]** iOS CoreBluetooth automatically starts pairing when an attribute access returns Insufficient Authentication or Encryption, then retries the request. Apps have **no API** to start pairing explicitly (Apple Accessory Design Guidelines, Bluetooth section: https://developer.apple.com/accessories/Accessory-Design-Guidelines.pdf).
  - Android also auto-bonds on `GATT_INSUFFICIENT_AUTHENTICATION/ENCRYPTION`, and additionally offers `BluetoothDevice.createBond()`.
  - Recommendation: put the encrypted permission on the **first** characteristic the app touches (for example a read-only "session-info" char or the CCC of the output char), so pairing happens up front rather than mid-session.
- **[H]** iOS: if the Pi forgets the bond, iOS fails with "Peer removed pairing information" until the user does "Forget This Device" in Settings. Apps cannot remove bonds on iOS. This is a UX cost of bonding.

### 2.2 L2CAP LE CoC socket security
- **[V]** `BT_SECURITY_LOW=1, MEDIUM=2, HIGH=3, FIPS=4` (`include/net/bluetooth/bluetooth.h:70-74`). The socket option accepts LOW..FIPS (`l2cap_sock.c:969-970`).
- **[V]** On an incoming LE credit-based connect, the kernel checks `smp_sufficient_security(hcon, pchan->sec_level)` and refuses with `L2CAP_CR_LE_ENCRYPTION` (if MEDIUM) or `L2CAP_CR_LE_AUTHENTICATION` (`l2cap_core.c:5271-5276`; ECRED at `:5497-5500`). The client must pair and retry.
- **[V]** `BT_SECURITY_FIPS` means SC **and** MITM **and** a 16-byte key: FIPS level is only reached via SC with MITM (`smp.c:602-612`, `2797-2800`), and `check_enc_key_size` requires 16 bytes when the pending level is FIPS (`smp.c:712-714`).
  - Per channel, it effectively enforces "SC-only + authenticated" for that PSM, without the adapter-wide `SecureConnections=only`.
  - It does **not** stop *other* services or legacy pairing on the adapter. Use main.conf `only` for that.
- **[H]** Android `createL2capChannel()` (the "secure" variant) requires encryption and auth and will bond. `createInsecureL2capChannel()` does not. iOS `openL2CAPChannel` reportedly triggers pairing on an auth-required refusal, but this needs on-device verification.

---

## 3. Threat model

| Threat | Just Works + login/PAM | Notes / mitigations |
|---|---|---|
| Passive eavesdropping (sniff pairing + session) | **Protected only with LE SC** (ECDH); legacy JW is crackable offline [H crackle] | `SecureConnections=only` [V] |
| Active MITM during pairing | **Not protected** (JW unauthenticated by definition) → attacker can read/inject the shell, capture password | Passkey (fresh, displayed) or app-layer key pinning; pairing window shortens exposure |
| Unauthorized connection by stranger | Can connect, pair (JW), and reach the `login:` prompt → attack surface = getty/login/PAM | Pairing window gate in `RequestAuthorization` [V mechanism]; bondable off after provisioning (`smp.c:1735-1737` returns PAIRING_NOTSUPP when not bondable and peer requests bonding) [V] |
| Password brute force over BLE | Limited by BLE latency + PAM | `pam_faildelay` / login.defs `FAIL_DELAY`, `LOGIN_RETRIES`, `pam_faillock` (deny=, unlock_time=) [H: standard man pages pam_faillock(8), login.defs(5)]; service-level backoff per peer identity address; disable root login on this tty (securetty/pam_securetty) |
| Replay of old session | LL encryption (AES-CCM, per-session SKD/IV) prevents replay [H spec] | App layer: SSH/Noise have nonces |
| Relay (extending range) | **Not prevented** by any BLE crypto [V NCC] | Require credentials every session; no proximity auto-login |
| Rogue server phishing client (BLESA-style) | Client may send password to fake Pi if it doesn't require encryption with bonded LTK [V BLESA] | App must require encrypted bonded link before showing/sending anything; better: server key pinning (SSH host key / Noise static key) |
| DoS by occupying the connection | A stranger can connect (no pairing needed to connect) and hold the link; advertising may stop | Disconnect connections that don't reach required security within N s (watch `Device1.Connected` + bonded state, call `Device1.Disconnect`) [H design]; allow multiple LE connections if controller supports it; cannot prevent RF jamming |
| Rogue reconnect with stolen/forged identity (RPA/IRK) | Attacker cannot resolve to a bonded identity without IRK; spoofing public identity address doesn't give LTK, encryption fails [H spec] | Require encrypted perms on all chars so a spoofed address without LTK gets nothing |
| Re-pairing / downgrade (BLERP, BLURtooth) | Risky if JustWorksRepairing=always or BR/EDR on | `JustWorksRepairing=never`, `ControllerMode=le`, `secure-*` perms [V/H as above] |
| Bond theft (phone or Pi compromised) | LTK in `/var/lib/bluetooth/<adapter>/<dev>/info` (root-only) [H] | App-layer auth (password/key) still required |

**Is "Just Works + login/PAM" enough?** It is enough against casual strangers and passive sniffing, provided SC-only is set. It is **not** enough against an active attacker present at first pairing (MITM leads to password capture and session hijack), and not enough against a rogue-server phishing attack on the client. The login password is sent through the pty as plain keystrokes, so link-layer security is the only protection unless an app-layer channel is added.

---

## 4. App-layer secure channel: SSH over the BLE byte pipe (recommended strongest tier)

### 4.1 Server side
- **[V]** `sshd -i` is inetd mode: one connection on stdin/stdout (`openssh/sshd.8:190-194`; flag parsed at `sshd.c:1400-1402`). Since the sshd/sshd-session split, the inetd path re-execs `sshd-session` with config passed over a socketpair (`sshd.c:1865-1870`), so it still works in current OpenSSH.
- **[V] Prior art:** OpenSSH's own regression suite runs its server this way, as `proxycommand ... sshd-log-wrapper.sh -i -f $OBJ/sshd_proxy` (`openssh/regress/test-exec.sh:887`). It also reuses that for PuTTY (`:826`) and Dropbear client tests (`:893-895`). SSH over an arbitrary non-socket pipe is therefore a tested configuration.
- **[H]** With a non-socket stdin, the remote IP is reported as "UNKNOWN" (`sshd-session.c` uses `ssh_remote_ipaddr`, `:1170-1173`). This is harmless; `Match Address` rules won't apply.
- **[H]** `dropbear -i` ("service program mode", inetd) does the same with a smaller footprint (man dropbear(8), https://manpages.debian.org/dropbear). sshd brings **PAM (UsePAM yes), login accounting, PTY allocation, window-change (`window-change` channel request), host keys, pubkey auth, rate limiting (MaxAuthTries, PerSourcePenalties [H: per-source penalties keyed on address which here is UNKNOWN, so all BLE clients share one bucket])**. Our daemon reduces to "BLE stream to `sshd -i` stdin/stdout"; no agetty and no PTY handling of our own.
- Bonus: the server-side BLE daemon needs no root for the PTY, though sshd -i itself must run as root for privsep and PAM.

### 4.2 Client libraries that accept a custom byte stream
- **[V] ConnectBot sshlib (Kotlin, Android):**
  - `interface Transport { suspend fun read(count); suspend fun write(data); close(); isConnected }`. The doc comment states "This interface allows SSH to work over any byte stream transport, not just TCP sockets" (`sshlib/src/main/kotlin/org/connectbot/sshlib/transport/Transport.kt:22-63`).
  - `fun interface TransportFactory { suspend fun create(): Transport }` (`transport/TransportFactory.kt:50-58`), plugged in via `SshClientConfig.transportFactory` (`SshClientConfig.kt:100-103`, `264-267`). The README lists "Pluggable transport layer (TCP via Ktor, or custom)".
  - It targets JVM 17 [V README]; Android compatibility needs desugaring or minSdk checks [H].
- **[V] SwiftNIO SSH (iOS):** `NIOSSHHandler` is a `ChannelDuplexHandler` with `InboundIn = ByteBuffer`, `OutboundOut = ByteBuffer` (`Sources/NIOSSH/NIOSSHHandler.swift:120-124`). It runs on any NIO Channel. Its own tests run client and server over in-memory `EmbeddedChannel`s (`Tests/NIOSSHTests/EndToEndTests.swift:26-44`).
  - For CoreBluetooth, the app would bridge `didUpdateValueFor` and `writeValue` (or an `L2CAPChannel` stream) into a custom Channel or a manually pumped EmbeddedChannel [H: engineering effort moderate].
  - Algorithms: curve25519-sha256 and ECDH-NIST KEX (`Key Exchange/ECDHCompatibleKey.swift:76,101`), host keys ssh-ed25519 and ECDSA (`SSHKeyExchangeStateMachine.swift:605`), AES-GCM only (`TransportProtection/AESGCM.swift:167,187`). All are compatible with OpenSSH defaults.
  - Host-key pinning is via `NIOSSHClientServerAuthenticationDelegate` (`SSHClientConfiguration.swift:21`).
  - SwiftNIO SSH is a protocol library. PTY and window-change requests are provided as channel requests; a terminal emulator is still needed on top (for example SwiftTerm) [H].
- **[H, not cloned]** Other options:
  - JSch: `Proxy` interface or `SocketFactory` returning custom streams.
  - Apache MINA SSHD: a custom `IoConnector` is possible but heavier.
  - libssh: `SSH_OPTIONS_FD` takes an fd, so a socketpair bridged to BLE would work.
  - libssh2: needs a socket fd, so use a socketpair too.
  - NMSSH and Citadel: Citadel is built on SwiftNIO SSH, so the transport would be swappable the same way.

### 4.3 Overhead over BLE
- **[V]** OpenSSH's default KEX is now `mlkem768x25519-sha256` (`openssh/myproposal.h`, `KEX_SERVER_KEX`). Its ephemeral shares are about 1.2 KB (client) and 1.1 KB (server) [H sizes from ML-KEM-768: 1184 B ek + 32, 1088 B ct + 32]. KEXINIT is about 1-1.5 KB each, plus the host key and signature, so the **total handshake is about 5-8 KB** [H estimate].
  - With curve25519-only (what SwiftNIO SSH negotiates) it is about 3-4 KB.
  - At realistic BLE GATT throughput (tens of KB/s with 2M PHY, DLE and MTU 185-517) that is roughly 0.1-0.5 s plus about 3-4 round trips × connection interval (15-50 ms). Acceptable.
- **[H, computed]** A one-byte keystroke as `SSH_MSG_CHANNEL_DATA`:
  - payload = 1 (type) + 4 (channel) + 4 (len) + 1 (data) = 10 B
  - chacha20-poly1305: 4 B length + padded block (1 + 10 + 5 pad = 16) + 16 B tag = **36 B**
  - aes-gcm: 4 + 28 (padded to 16-multiple with ≥ 4 pad) + 16 = **48 B**
  - The echo comes back the same size. That is about 35-50 B per keystroke each way versus 1 B raw. It fits in **one** ATT write or notification only if the negotiated ATT MTU is ≥ ~51. With the default MTU 23 (20 B payload) every keystroke fragments into 2-3 PDUs, so MTU exchange is mandatory (iOS typically 185; Android `requestMtu(517)`).
- **[V] Keystroke-timing chaff:**
  - OpenSSH ssh(1) `ObscureKeystrokeTiming` (default on since 9.5) sends keystrokes at fixed intervals of "a few tens of milliseconds and will send fake keystroke packets for some time after typing ceases" (`openssh/ssh_config.5:1517-1529`).
  - ConnectBot sshlib implements the same with a default `obscureKeystrokeTimingIntervalMs = 20L` (`SshClientConfig.kt:179`), using `SSH_MSG_PING` chaff (`client/SshConnection.kt:3117-3121`), active only when a PTY is granted and the server supports ping (`client/SessionChannel.kt:177-178`).
  - Over BLE this means about 50 packets/s × ~36-48 B, roughly 2 KB/s plus extra radio wakeups, for a while after each keystroke. **Set it to 0 for BLE**: its timing-privacy value is limited because BLE connection events already quantise timing [H].

### 4.4 Noise alternative
- Noise_XX (mutual, TOFU) or Noise_IK (client knows the pinned server static key, for example from a QR code or first-use) over the same byte pipe. The overhead is lower (16 B tag + 2 B length per message; 1-RTT handshake of about 100-200 B with 25519) [H from Noise spec, https://noiseprotocol.org/noise.html].
- The downside is that we then reimplement what SSH gives for free: user auth (still need PAM: password inside the channel, or pubkey-to-account mapping), PTY and window-change framing, rekeying and audit. Libraries exist, such as noise-java (rweather) and Swift Noise ports, but are less battle-tested [H].
- Noise is worth it only if handshake bytes or code size matter (they do not much on a Pi).

---

## 5. Recommended security tiers

**Tier 0: Dev only.** No pairing, open GATT, agetty/login. Password in cleartext over the air. Never ship this.

**Tier 1: MVP. "Encrypted link + PAM"**
- main.conf: `SecureConnections = only`, `JustWorksRepairing = never`, `ControllerMode = le` (if BR/EDR unused) [H option], `Privacy = device` (optional).
- A custom default agent with capability **NoInputNoOutput**. `RequestAuthorization` accepts **only inside a pairing window** opened by a physical button, a `ble-term pair` CLI, or for the first N minutes after boot (configurable). Outside the window, reject. This is the [V] mechanism in `device.c:7972`.
- GATT: `encrypt-read`, `encrypt-write` and `encrypt-notify` on **all** terminal characteristics. For L2CAP CoC, `BT_SECURITY_MEDIUM`.
- Login via agetty/login/PAM with `pam_faillock` and `FAIL_DELAY`. Disallow root on this tty.
- Client app refuses to send any byte before the link is encrypted (protects against BLESA-style fake servers).
- Residual risk: an active MITM during the (short) pairing window.

**Tier 2: Stronger. "Authenticated pairing + bond allow-list"**
- Agent capability **DisplayOnly**. `DisplayPasskey` writes the kernel-generated, fresh passkey to the console, serial, journald (`journalctl -u ble-term`), an LED blink pattern, e-paper or an attached display. GATT flags `secure-*` (SC + MITM + 16-byte key), or L2CAP `BT_SECURITY_FIPS`.
- After provisioning, set **Bondable off** (`btmgmt bondable off` or `Adapter1.Pairable=false`). New bond attempts then get `PAIRING_NOTSUPP` [V `smp.c:1735`], and only existing bonds can reconnect. Re-open pairing only by local action.
- Admin CLI to list and remove bonds (`bluetoothctl remove` / `Adapter1.RemoveDevice`), because JW re-pairing is disabled and lost bonds need manual cleanup on both sides (iOS: "Forget device").
- **Headless UX problem:** a passkey is only useful if the person pairing can read it, which requires console, SSH (unavailable by design), a display or LEDs. A static passkey is not possible on Linux (1.1) and would be weak anyway (1.4). LE OOB pairing is not exposed to third-party apps on iOS (no CoreBluetooth API) or on Android (`createBondOutOfBand` is a @SystemApi) [H], so a QR-code OOB flow is not available at the BLE layer.

**Tier 3: Strongest. "SSH over the BLE pipe" (recommended target architecture)**
- Server: BLE daemon pipes a stream to `sshd -i` (or `dropbear -i`). Client: ConnectBot sshlib (`Transport`) or SwiftNIO SSH (custom Channel).
- **Host-key pinning:** a QR code or fingerprint printed or shown on the device, or TOFU like `known_hosts`. This moves the authentication anchor from BLE pairing (which is weak on headless devices) to SSH host keys. A QR code works fine for headless devices, since the fingerprint is static and safe to print, unlike a passkey.
- User auth: pubkey from the phone keystore (Android Keystore or iOS Secure Enclave via ECDSA P-256 [H]), or password through PAM. sshd provides PAM, PTY and window-change.
- Keep BLE Tier 1 underneath (encrypt plus pairing window) to raise the bar for DoS and pre-auth surface. Its MITM weakness no longer matters for confidentiality.
- Costs: about 5-8 KB handshake; about 36-48 B per keystroke (needs MTU > 51); disable keystroke chaff; one more dependency in each app.
- This also defeats relay attacks on credentials (the relayer sees only ciphertext) but **not** relay of presence. That is acceptable because no proximity-based auth is used.

**Summary of tier choice for a headless Pi:** Tier 1 + Tier 3 (Just Works with a button-gated pairing window, plus SSH with host-key pinning) gives the best security per unit of UX effort. Tier 2's passkey needs some local output channel and adds little once Tier 3 is in place.
