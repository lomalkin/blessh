> Research working notes (2026-10-24): client platforms beyond the Android app and the Linux ProxyCommand. Not yet acted on; decisions go to backlog.md.

# F. ble-term: client platforms beyond the ConnectBot fork and the Linux ProxyCommand

Research date: 2026-10-24. Labels: **[V]** verified from a primary source (spec, vendor docs, upstream source); **[S]** secondary source (issue threads, forums, READMEs); **[H]** inference that needs a test on hardware. Mobile BLE API facts come from `docs/research/D-mobile-os-ble-apis.md` and are not repeated here.

## Executive summary

* **A Web Bluetooth client works under the current v1 spec. The protocol and server need no changes.** It runs in Chrome/Edge on Windows, macOS, ChromeOS and Android, and on Linux only behind a flag. It does not run in Safari or Firefox, or in any iOS browser except third-party wrappers such as Bluefy. The one real gap is that JavaScript cannot read the ATT MTU. The spec still allows a safe frame size: send 20-byte frames, then raise the size to the largest TX notification received. ATT_MTU is a single per-bearer value used in both directions, so this stays within `min(ATT_MTU−3, 500)`. Uplink traffic is mostly keystrokes, so small frames cost little.
* **The SSH side in the browser is the larger job.** No mature pure-JS SSH library supports our ed25519 host key over an arbitrary stream. For example, `@microsoft/dev-tunnels-ssh` has no ed25519 and no curve25519. The workable routes are a WASM SSH core: Go `x/crypto/ssh` (proven in the browser by sshterm), Google's WASM OpenSSH (nassh/wassh), or russh (its CI builds only wasm32-wasip1). Add xterm.js and IndexedDB storage on top. Estimate: about 3–5k LOC of our own code and **5–8 person-weeks** to production quality. The main risk is that whoever controls the web origin controls the keys.
* **The cheapest new platforms are macOS and Windows ProxyCommand helpers**, the analogue of `ble-term-connect`. The stock OpenSSH client then does all the SSH work. Estimate: 1–2 weeks each, or about 2–3 weeks for both on one Rust codebase.
* **iOS/iPadOS is feasible** with CoreBluetooth, SwiftNIO SSH and SwiftTerm. Estimate: 6–10 weeks. Closed apps (Termius, Blink, JuiceSSH) cannot be extended.
* **An Android localhost bridge app (BLE ↔ 127.0.0.1:port) is cheap.** It lets any Android SSH client (Termux `ssh`, JuiceSSH, Termius) connect without a fork.
* **Protocol and server changes worth making are all additive:**
  * Document the "derive frame size from received notifications" rule and write-with-response as an allowed RX path. The server already supports both.
  * Optionally add an `mtu=` key to INFO.
* **Not worth doing now:** L2CAP CoC (it widens no platform; Web Bluetooth and WinRT lack it), any relay or WebSocket, and adding a second host key.

## 1. Web Bluetooth

### Platform matrix

| Browser / OS | Status | Source |
|---|---|---|
| Chrome/Edge, Windows 10 1703+ | Supported (GATT since Chrome 70, Edge 79+) | [V] [implementation-status.md](https://github.com/WebBluetoothCG/web-bluetooth/blob/main/implementation-status.md) |
| Chrome/Edge, macOS | Supported | same |
| Chrome, ChromeOS | Supported. **This is the only practical ChromeOS route**, see section 2 | same |
| Chrome, Android 6+ (also Samsung Internet, Opera) | Supported | same; [caniuse](https://caniuse.com/web-bluetooth) |
| Chrome, Linux | **"Partially implemented, not supported"**, needs `chrome://flags/#enable-experimental-web-platform-features`. Linux already has `ble-term-connect`, so this does not matter | [V] implementation-status.md |
| Firefox (all platforms) | No; no near-term plans | [V] same; caniuse |
| Safari macOS/iOS; Chrome on iOS (WebKit) | No | [V] same |
| iOS third-party browsers (Bluefy, WebBLE, iOSWebBLE extension) | They polyfill `navigator.bluetooth` over CoreBluetooth. Not a product basis: closed source, and behaviour and backpressure are unknown | [S] same |
| Android WebView | No | [V] same |

### API details that matter for BleStream

* **Secure context.** All interfaces are `[SecureContext]`, so the page needs HTTPS or localhost [V] ([spec source](https://github.com/WebBluetoothCG/web-bluetooth/blob/main/index.bs)). A user gesture is needed for `requestDevice()`.
* **Discovery.**
  * The daemon already puts the service UUID in the advertisement (`daemon.rs` `advertise()`: `service_uuids: [SERVICE_UUID]`, plus `local_name`). So `requestDevice({filters:[{services:[SVC]}]})` works.
  * A `namePrefix` filter needs `optionalServices:[SVC]` to grant access to the service.
  * A 128-bit UUID plus flags takes 21 of the 31 legacy advertising bytes, which leaves little room for the name. Check with `btmon` whether BlueZ truncates the name or moves it to the scan response [H]. Chrome scans actively, so either placement works.
* **GATT blocklist.** Our UUIDs are custom, so the blocklist does not affect them [V] ([gatt_blocklist.txt](https://github.com/WebBluetoothCG/registries/blob/master/gatt_blocklist.txt)).
* **Write size.**
  * `writeValueWithoutResponse()` rejects values over 512 bytes [V spec]. It gives no way to learn the MTU.
  * Writing more than MTU−3 behaves differently per platform:
    * BlueZ "command" writes fail with "Failed to initiate write" [S] ([BlueZ patch thread](https://patches.linaro.org/patch/502985)).
    * Other stacks may truncate or reject [S].
  * Therefore never exceed the known-safe size.
* **MTU.**
  * There is no MTU API. [whatwg/bluetooth#383](https://github.com/WebBluetoothCG/web-bluetooth/issues/383) is open, and a `getMTU()` prototype by an outside contributor was announced in June 2026 [S].
  * Chrome on Android has requested a large MTU since 2024; a 512 MTU was observed [S, same issue].
  * Anecdotes: macOS Chrome only worked up to MTU 104, while Windows worked up to 517 [S].
* **How BleStream copes, with no spec change.**
  1. Start with 20-byte frames. HELLO (13 bytes) and CREDIT (7 bytes) fit.
  2. ATT_MTU is negotiated once per bearer and applies in both directions. Any TX notification of length *n* therefore proves ATT_MTU ≥ *n*+3.
  3. The server fragments the SSH banner and KEXINIT into full-size frames, so the client learns `min(MTU−3, 500)` within the first round trip.
  4. Raise the frame size to the largest notification seen.
* **Caveat (EATT).** The bearers could have different MTUs; Chrome on Linux/BlueZ is the only client that might use EATT [H]. Clamp the frame size to 244 unless the client has seen ≥ 2 large notifications. Optionally use the INFO `mtu=` key from section 3.
* **Backpressure.**
  * The write promise resolves when the platform write completes. The spec says the UA *may* reject with `NetworkError` while it is busy ("sites need to serialize their use of this API", Issue 188) [V spec].
  * So all GATT operations (HELLO, DATA, CREDIT, INFO read) go through one awaited queue. That gives one write in flight, which is real backpressure on Android, where Chrome waits for `onCharacteristicWrite`.
  * **Risk [H]:** Chromium on macOS may not gate on `canSendWriteWithoutResponse`, which would mean silent drops. A silent drop is fatal for BleStream: there is no retransmission, so the next frame is out of order and the link closes.
  * Mitigation: use `writeValueWithResponse()` on macOS, which RX already allows (see section 3). Make "with response" a runtime toggle and stress-test both modes.
* **Notifications.**
  * Each notification queues a task on the Bluetooth task source [V spec], so they are delivered in order.
  * The browser applies no backpressure, but BleStream credits bound the queue.
  * Add the `characteristicvaluechanged` listener **before** `startNotifications()`. The server sends HELLO as soon as both characteristics are in use.
* **Permissions and reconnect.**
  * `getDevices()` and persistent permissions are still behind flags on every platform [V implementation-status]. The user picks the device in the chooser on each page load: one click, acceptable.
  * After a disconnect, the same `BluetoothDevice` object can be reconnected within the page session. The client must call `startNotifications()` again (spec note).
* **Background.**
  * Scanning stops when the page is hidden [V spec §"Handling Visibility Change"]. An existing connection keeps working.
  * Desktop Chrome throttles timers but still dispatches events [H]. Chrome on Android can freeze or discard a background tab [H]. The server ends the session on disconnect, so tmux on the server is the remedy.

### SSH in the browser

Server requirements, from `packaging/sshd_config`:
* Host key: ssh-ed25519 only.
* KEX: OpenSSH 9.2 offers curve25519-sha256 (sntrup761 is excluded); OpenSSH 10 adds mlkem768x25519-sha256.
* Ciphers: chacha20-poly1305 or aes-gcm.
* Authentication: password through PAM, keyboard-interactive, or publickey.

| Option | Transport-agnostic | ed25519 / curve25519 | Maturity | Notes |
|---|---|---|---|---|
| Go `x/crypto/ssh` → WASM (as in [sshterm](https://github.com/c2FmZQ/sshterm), MIT) | yes (`net.Conn` wrapper) | yes; mlkem768x25519 in recent Go [S] | high (library); sshterm small | About 5–10 MB WASM. sshterm already has xterm.js, IndexedDB, known_hosts and WebAuthn `sk` keys, but only a WebSocket transport. **Recommended starting point.** |
| Google WASM OpenSSH ([libapps ssh_client/wassh](https://chromium.googlesource.com/apps/libapps), BSD) | via JS socket shim | full OpenSSH 10.x | very high | Built for the Secure Shell extension. Reusing it outside the extension and plugging in a custom socket is undocumented [S]. Large. |
| russh → WASM | yes | yes, plus mlkem768x25519 | high (native) | CI builds only `wasm32-wasip1` with `ring` [V, russh `rust.yml`]. A browser build (`wasm32-unknown-unknown`, no tokio runtime) is unproven [H]. Advantage: we could compile `protocol.rs`/`link.rs` into the same module. |
| `@microsoft/dev-tunnels-ssh` (TS) | yes | **no** (both "under consideration") | medium | Would need an ECDSA host key on the server. Rejected. |
| `ssh2` (Node) | Node only | — | — | Needs Node crypto and net. Rejected. |

* **WebCrypto.** X25519 has shipped since Chrome 133 and Ed25519 since Chrome 137 [S] ([Igalia](https://blogs.igalia.com/jfernandez/?p=1913), [IPFS blog](https://blog.ipfs.tech/2025-08-ed25519/)). A WASM SSH core does not need them, but native Ed25519 allows a **non-extractable** user key stored in IndexedDB.
* **Better client keys:** FIDO via WebAuthn with OpenSSH's `webauthn-sk-*` signature format, as sshterm and nassh do [S].
* **Host key pinning.** Use trust on first use with a fingerprint prompt, and store known_hosts in IndexedDB keyed by the INFO `name`. The browser never sees the MAC address and gives a per-origin device id.
* **Terminal.** xterm.js; mobile input issues are listed in `B-terminal-engines.md`.

### Size and risks

* **Our code:**
  * BleStream link in TypeScript, about 400 LOC, or `link.rs` compiled to WASM.
  * GATT glue and queue: about 300 LOC.
  * SSH glue: 1–2k LOC.
  * UI, storage and PWA: 1–2k LOC.
* **Effort:** 5–8 person-weeks including hardware testing on Windows, macOS, ChromeOS and Android.
* **Risks:**
  1. **Origin = trust root.** Whoever serves the JS reads every keystroke and the private keys. Mitigations: self-host on a domain we control, install as a PWA, ship a strict CSP, use non-extractable or WebAuthn keys, and possibly a signed or reproducible bundle. This is weaker than a store-signed native app.
  2. **Supply chain** of npm and Go modules.
  3. **Offline use.** A service worker cache is needed, because the device often has no network (that is the point of BLE).
  4. **Silent write drops on macOS Chrome** [H].
  5. **Throughput.** Downlink is bounded by the server (about 30–100 KB/s). Uplink with one serialized write and small frames could be about 1–10 KB/s [H]; fine for typing, slow for `scp`.
  6. **Storage eviction.** IndexedDB can be evicted. Request `navigator.storage.persist()` and offer export.

## 2. Other platforms

| Platform | Approach | Works under current v1 spec? | Effort | Main risks / limits |
|---|---|---|---|---|
| **macOS** | ProxyCommand helper in Rust, using CoreBluetooth through `objc2-core-bluetooth` or `btleplug`, run as `ssh -o ProxyCommand='ble-term-connect --name pi'` | Yes | 1–2 wk | See notes |
| **Windows 10/11** | ProxyCommand helper using WinRT (`windows` crate); Win32-OpenSSH supports ProxyCommand | Yes | 1–2 wk (+1 wk if shared with macOS via `btleplug`) | See notes |
| **iOS / iPadOS** | Native app: CoreBluetooth + [swift-nio-ssh](https://github.com/apple/swift-nio-ssh) + SwiftTerm | Yes | 6–10 wk + App Store review | See notes |
| **ChromeOS** | Web Bluetooth client (section 1). The Android ConnectBot fork may run under ARC [H] | Yes | covered by the web client | The Linux container (Crostini) has no Bluetooth access [H], so `ble-term-connect` is not usable there |
| **Linux desktop** | exists (`ble-term-connect`) | — | — | — |
| **Android, any SSH app** | Small "BLE bridge" app: foreground service that listens on 127.0.0.1:N and pumps BleStream; `ssh -p N user@127.0.0.1` from Termux, JuiceSSH, Termius | Yes | 2–3 wk (reuse the ConnectBot fork's BleStream code) | See notes |

**macOS notes**
* No MAC address: select by INFO or advertised name, or by the CoreBluetooth UUID. That UUID is per Mac, not portable [V, D-doc].
* TCC Bluetooth prompt: a CLI tool needs an embedded `Info.plist` with `NSBluetoothAlwaysUsageDescription`, and the permission is attributed to the terminal app [H].
* Gate writes on `canSendWriteWithoutResponse`; `btleplug` may not do this [H]. Fallback: write with response.
* MTU comes from `maximumWriteValueLength(.withoutResponse)`.

**Windows notes**
* `BluetoothLEDevice.FromBluetoothAddressAsync` connects unpaired.
* `GattSession.MaxPduSize` gives the MTU (read-only, with a change event) [V] ([docs](https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.genericattributeprofile.gattsession)).
* Windows caches GATT databases: use `BluetoothCacheMode.Uncached` for the first discovery.
* Write-without-response above MTU−3 errors out [S].
* No public LE CoC API [H].
* Quote paths carefully in `ProxyCommand`.

**iOS / iPadOS notes**
* swift-nio-ssh: x25519 KEX, ed25519 host key, AES-GCM, password and publickey authentication. No keyboard-interactive, so PAM prompts beyond a plain password fail.
* Alternative: libssh2 over a `socketpair` (gives keyboard-interactive).
* `bluetooth-central` background mode. Force-quit ends the session (QA1962).
* No MAC: pin by INFO name plus host key.
* L2CAP CoC is possible but unproven against BlueZ (D-doc).
* Closed apps (Termius, Blink, Prompt) cannot be extended. Blink is GPLv3 and could be forked, but that is a heavy route.
* A localhost bridge does not work: iOS suspends the bridge app while the SSH app is in the foreground [H].

**Android bridge notes**
* Any local app can connect to the localhost port. That is no worse than anyone in radio range, and host key pinning still protects against a local app squatting the port.
* Needs a foreground service of type `connectedDevice`.
* Termux:API has no GATT support, so a pure Termux ProxyCommand is impossible without such a helper.

**Shared desktop option.** One Rust crate already holds `link.rs`, `bridge.rs` and `connect.rs`. Splitting out a transport trait with BlueZ, CoreBluetooth and WinRT backends gives one `ble-term-connect` for three OSes. `btleplug` covers all three, but check that it exposes MTU and write-without-response flow control. If it does not, use thin per-OS backends.

## 3. Candidate protocol and server changes

Facts checked in code:
* RX already declares **write** as well as write-without-response (`daemon.rs`).
* BlueZ sends write *requests* into the AcquireWrite socket too [V] ([BlueZ 5.66 `gatt-database.c` `chrc_write_cb`](https://github.com/bluez/bluez/blob/5.66/src/gatt-database.c), `if (chrc->write_io) sock_io_send(...)`).
* The receiver does not check incoming frame size against its own MTU (`protocol.rs` `decode`).

| Candidate | Benefit | Cost / fragility | v1 compatibility | Recommendation |
|---|---|---|---|---|
| Service UUID in the advertisement | Filtered discovery (Web Bluetooth, iOS background scan) | none | already frozen and implemented | **Done.** Only verify that the name is not lost to the 31-byte limit (btmon) |
| Write-with-response as a sanctioned RX path | Fallback where write-without-response is lossy or unflow-controlled (macOS Chrome, btleplug) | Zero on the server. About half the uplink throughput | No wire change; a doc change in PROTOCOL.md "Client rules" | **Yes**: document it, with frames ≤ MTU−3. Long (prepared) writes would reach the socket as separate chunks and break framing [H, untested], so forbid them. Add an e2e test |
| Frame size from observed notifications | Safe sizing without an MTU API (Web Bluetooth, any wrapper) | none | spec clarification only | **Yes**: add to PROTOCOL.md: "clients that cannot read ATT_MTU start at 20 and may raise it to the largest received notification length" |
| INFO `mtu=<n>` (from the read request's MTU; `bluer` passes it) | Immediate frame size for MTU-blind clients | tiny; INFO stays static except this key | INFO extension allowed (unknown keys ignored) | **Optional, low priority.** EATT bearers may differ, so clients still take min(INFO, observed) |
| Explicit frame-size field in HELLO trailing bytes (flag-guarded) | Each side states its maximum receivable frame | small. Adds nothing over the two rows above, since the receive path already accepts any size up to 512 | allowed extension | **No** for now |
| Host key fingerprint in INFO | UX hint | Unauthenticated; users may treat it as a trust anchor | allowed | **No** |
| L2CAP CoC transport | Real backpressure and throughput on iOS/Android/Linux | New code path, unproven interop, iOS background unknown | v2 or optional upgrade via INFO/PSM characteristic | **Defer.** Widens no platform: Web Bluetooth and WinRT have no CoC |
| ECDSA host key (for libraries without ed25519) | Enables dev-tunnels-ssh-style libraries | Second key to protect. Clients may re-pin depending on order | contract keeps the ed25519 key | **No**; the WASM SSH cores support ed25519 |
| Relay / WebSocket bridge | none for BLE-local use | adds a network attack surface | n/a | **No**; keep the web client as static files only |
| Bigger handshake timeout | none: the chooser runs before connect | — | not in contract | No |

## 4. Open questions / tests

1. Chrome on macOS: does `writeValueWithoutResponse()` drop writes under load? Run a stress test with sequence checking. Also check the reported MTU ≤ 104 limit (#383).
2. Chrome on Android/Windows/ChromeOS: the actual negotiated MTU against BlueZ, and uplink throughput with serialized writes.
3. BlueZ server: a write request ≤ MTU−3 through AcquireWrite (expected to work). A prepared or long write (expected to break; confirm and then forbid in the doc).
4. Whether russh builds for `wasm32-unknown-unknown` with a single-threaded executor, or which Go WASM size and performance is acceptable on a mid-range phone.
5. Whether wassh (OpenSSH WASM) can be embedded in a plain PWA with a custom socket.
6. Advertisement layout: is the full host name present (adv or scan response) on Bookworm and Trixie BlueZ?
7. ChromeOS: does the ConnectBot fork's BLE work under ARC++/ARCVM?
8. macOS CLI TCC behaviour for an unbundled Rust binary using CoreBluetooth.
