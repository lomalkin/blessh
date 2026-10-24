# BLE Terminal Architecture Research for Linux

Date: 2026-10-24. Status: stage 1 (research) complete; operator decisions pending (section 10).

Working notes with full quotations, commit SHAs and line numbers are moved to the appendices:

| Appendix | Topic |
|---|---|
| [research/A-mobile-ble-uart-apps.md](A-mobile-ble-uart-apps.md) | Mobile BLE UART/terminal apps, "shell over BLE" projects |
| [research/B-terminal-engines.md](B-terminal-engines.md) | Terminal engines and mobile terminal/SSH clients |
| [research/C-linux-server.md](C-linux-server.md) | BlueZ D-Bus, L2CAP CoC, libraries, PTY/login/PAM, privileges, packaging |
| [research/D-mobile-os-ble-apis.md](D-mobile-os-ble-apis.md) | CoreBluetooth and Android BLE limitations at the system API level |
| [research/E-security.md](E-security.md) | Pairing/bonding on a headless device, threat model, SSH/Noise over BLE |

Notation: **[V]** — verified against source code or primary documentation (link given); **[S]** — secondary source (vendor blog, forum, third-party measurements); **[H]** — hypothesis/estimate, requires an experiment.

Revisions examined (shallow clone): BlueZ `a0580dad` (5.87-dev), BlueZ tags 5.55/5.66–5.69/5.82; Linux `17bb0e2f`; bluer `57ec7045`; termux-app `8629e632`; connectbot `0bea3773`; connectbot/termlib `56934dd1`; connectbot/sshlib `9b80e3da`; SwiftTerm `c3436be3`; swift-nio-ssh `149f32ff`; xterm.dart `d35ba2cc`; blink `a90b4423`; libvterm `934bc2fb`; OpenSSH `26b1363c`; kable `ad88767b`; SimpleBluetoothLeTerminal `f322c06c`; nRF Toolbox (Android `c7f8e443`, iOS `d419fb24`); Bluefruit LE Connect; asteroid-ble-shell `36d19c6a`; linkr-bee `8ea74735`; sorah/bluetooth-getty `c98973e9` and others (full list in the appendices).

---

## 1. Executive summary

1. **There is no ready-made solution.** None of the open-source mobile apps found does full VT/xterm emulation over BLE: all BLE UART clients (nRF Toolbox, Bluefruit, SimpleBluetoothLeTerminal, etc.) are a log of lines with an input field [V, app. A §1]. Server-side "shell over NUS" for Linux exists (pi-loop, ble_tty, asteroid-ble-shell, coBlue), but none of them has a terminal resize channel, flow control, or a mobile client; two of them hand out a root shell without authentication [V, app. C §4].
2. **Mature building blocks exist, and they fit together well.** Terminal engines with a transport-independent API: **SwiftTerm** (iOS, MIT) and **ConnectBot termlib** / Termux terminal-emulator (Android, Apache-2.0) [V, app. B]. SSH libraries with a pluggable byte transport: **ConnectBot sshlib** (`Transport`/`TransportFactory`) and **SwiftNIO SSH** (works over any NIO Channel) [V, app. E §4.2]. On the server: `sshd -i` works over an arbitrary stdin/stdout stream, and this is routinely tested in the OpenSSH regression suite [V].
3. **Key finding of the research:** the optimal architecture is **BLE as a "reliable byte pipe" + the SSH protocol on top of it**: on the server, a per-connection `sshd -i` (PAM, PTY, controlling TTY, window-change, session accounting, host keys — all out of the box); on the clients, existing "terminal engine + SSH library" pairs that are already integrated with each other in ConnectBot (Android) and in the SwiftTerm demo app (iOS). Our own code comes down to: (a) a BLE daemon on Linux with a GATT service and credit-based flow control, (b) a BLE transport for sshlib/SwiftNIO SSH on the clients. This maximizes reuse and at the same time solves the main security problem of a headless device (BLE pairing without a display does not protect against MITM; SSH host key pinning does).
4. **Transport:** the baseline is a **custom GATT service** (write-without-response input, notify output, in-band framing with credits). Raw NUS is unsuitable as the main protocol (no resize, no backpressure, no control channel). **L2CAP CoC** is technically better on Linux (real credit-based backpressure, per-client sockets, no privileges), but iOS/Android ↔ BlueZ CoC compatibility is not publicly confirmed, and CoC behavior in the background on iOS is undocumented — so CoC is planned as an optional speed-up after an experiment, with fallback to GATT.
5. **Main technical constraints:** (a) BlueZ has no backpressure for notifications (unbounded `att->write_queue`), and through AcquireWrite, write-without-response packets are silently dropped on EAGAIN — our own credit protocol is needed [V]; (b) BlueZ 5.66 (Debian/RPi OS Bookworm) cannot do per-device fds for AcquireNotify/AcquireWrite — it supports only one client per characteristic; from 5.69 on (Trixie: 5.82) — per-device [V]; (c) on iOS, the MTU cannot be set, the connection interval is ≥ 15 ms, and pairing cannot be initiated programmatically [V]; (d) on Android — one GATT operation in flight [V].
6. **The server does not require** changes to `main.conf`, `--experimental`, or root for the BLE part, and does not monopolize the adapter: GATT/advertising registration via BlueZ D-Bus is available to an unprivileged user and disappears when the process stops [V].
7. **The recommended MVP** can be verified without mobile apps: BLE daemon on the Pi + a desktop "ProxyCommand" client → plain `ssh -o ProxyCommand=...` gives a full terminal (vim/htop/tmux) over BLE already at the first stage. Then — a ConnectBot fork (Android) and an app on SwiftTerm + SwiftNIO SSH (iOS).
8. This deviates from the letter of the brief in one point: instead of `agetty`/`login`, the session is started by `sshd` (which itself uses PAM and a standard PTY). The alternative with `login`/PAM and our own protocol has been worked out (section 7.1) and remains feasible; the choice is left to the operator (question 1).

---

## 2. Projects found

Activity — per `gh api` as of 2026-10-24. "Suitability": ★★★ — take as the base, ★★ — use partially/as a library, ★ — reference only, ✗ — do not use.

### 2.1 Mobile clients and BLE libraries

| Project | Platform | License [V] | Activity | What's inside | Suitability |
|---|---|---|---|---|---|
| [connectbot/connectbot](https://github.com/connectbot/connectbot) | Android, Kotlin/Compose | Apache-2.0 | 3.5k★, push 2026-10-23 | Full SSH client: termlib + sshlib, hosts, extra-keys, background service; `AbsTransport`; sshlib is already used with a non-standard `transportFactory` for jump hosts (`transport/SSH.kt:976-1011`) | ★★★ base of the Android client (fork + BLE transport) |
| [connectbot/termlib](https://github.com/connectbot/termlib) | Android, Compose + libvterm (JNI) | Apache-2.0 + MIT | 28★, 0.3.x, active | `writeInput` / `onKeyboardInput` / `onResize` — not tied to a transport (`TerminalEmulator.kt:82-275`) | ★★★ (via ConnectBot) |
| [connectbot/sshlib](https://github.com/connectbot/sshlib) | Kotlin/JVM | Apache-2.0 | active, 0.6.x | `interface Transport` "any byte stream transport" (`transport/Transport.kt:22-63`), `TransportFactory` (`:50-58`) | ★★★ |
| [termux/termux-app](https://github.com/termux/termux-app) terminal-emulator / terminal-view | Android, Java | Libraries Apache-2.0 (per `LICENSE.md`; subdirectories have no license files of their own), the rest GPLv3; `ExtraKeysView` — GPLv3 | 63k★, push 2026-09 | Best IME, Unicode 15; `TerminalSession` is `final` and tied to JNI/PTY (`TerminalSession.java:31,103-131`) | ★★ fallback engine (patch ~150–300 lines) |
| [migueldeicaza/SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) | iOS/macOS, Swift | MIT | 1.7k★, v1.99.1 2026-10-21 | `feed(byteArray:)`, `send(data:)` delegate, `sizeChanged`; Esc/Ctrl bar; Unicode 17; esctest; SSH terminal demo on swift-nio-ssh (`TerminalApp/iOSTerminal/UIKitSshTerminalView.swift`); La Terminal and Secure ShellFish are built on it | ★★★ base of the iOS client |
| [apple/swift-nio-ssh](https://github.com/apple/swift-nio-ssh) | Swift | Apache-2.0 | Apple, active | `NIOSSHHandler` with `ByteBuffer` in/out (`NIOSSHHandler.swift:120-124`), tests on in-memory channels; curve25519, Ed25519/ECDSA, AES-GCM; pinning via delegate | ★★★ |
| [JuulLabs/kable](https://github.com/JuulLabs/kable) | Kotlin Multiplatform | Apache-2.0 | 1.2k★, active | Correct WNR flow control on both OSes (`CBPeripheralCoreBluetoothPeripheral.kt:213-229`, `BluetoothDeviceAndroidPeripheral.kt:195-211`) | ★★ (option for a shared BLE layer; pattern reference) |
| [nordicsemi/Kotlin-BLE-Library](https://github.com/nordicsemi/Kotlin-BLE-Library), [Android-BLE-Library](https://github.com/nordicsemi/Android-BLE-Library) | Android | BSD-3 | active | Request queues, MTU, split | ★★ alternative BLE layer on Android |
| [nordicsemi/IOS-CoreBluetooth-Mock](https://github.com/nordicsemi/IOS-CoreBluetooth-Mock) | iOS | BSD-3 | active | CoreBluetooth mock for tests/simulator | ★★ for tests |
| [kai-morich/SimpleBluetoothLeTerminal](https://github.com/kai-morich/SimpleBluetoothLeTerminal) | Android | MIT | 345★, 2026-04 | Write queue driven by `onCharacteristicWrite` (`SerialSocket.java:349-421`), Telit TIO credits (`:506-645`); UI is a log of lines | ★ reference (GATT state machine, credits) |
| [nordicsemi/Android-nRF-Toolbox](https://github.com/nordicsemi/Android-nRF-Toolbox), [IOS-nRF-Toolbox](https://github.com/nordicsemi/IOS-nRF-Toolbox) | Android / iOS | BSD-3 | active | UART profile: chat log; iOS drops non-UTF-8 packets and trims whitespace (`UARTViewModel.swift:172-175`) | ★ reference; as a terminal ✗ |
| Adafruit Bluefruit LE Connect ([iOS](https://github.com/adafruit/Bluefruit_LE_Connect_v2), [Android](https://github.com/adafruit/Bluefruit_LE_Connect_Android_V2)) | iOS / Android | MIT | iOS — 2024, Android — archived | WNR in a loop without `canSendWriteWithoutResponse` (`BlePeripheral.swift:506-513`) — antipattern | ✗ |
| Serial Bluetooth Terminal (Kai Morich) | Android | proprietary | Play Store | "line-oriented terminal" | ✗ (closed) |
| [johnhowe/BlueTerm](https://github.com/johnhowe/BlueTerm) | Android | no LICENSE | 2014 | The only mobile VT100 over Bluetooth found — but Classic SPP | ✗ (history) |
| [TerminalStudio/xterm.dart](https://github.com/TerminalStudio/xterm.dart) | Flutter | MIT | last release 2024-02, 109 issues | `String`-based API, Unicode 11 | ✗ (slowing down) |
| flutter_blue_plus | Flutter | **custom license** (commercial for for-profit, telemetry at build time) | active | — | ✗ (license); alternative universal_ble (BSD-3) |
| [blinksh/blink](https://github.com/blinksh/blink) | iOS | GPL-3.0 | 7k★ | hterm in WKWebView, native transport | ★ reference; an App Store fork is a legal risk |
| xterm.js (in a WebView, not Web Bluetooth) | both | MIT | 21k★, 6.0 | Best emulation; open issues on mobile input (#5377, #3600, #3727, #5108) | ★ fallback option for a shared UI |
| libvterm, alacritty_terminal, termwiz, libghostty-vt | C / Rust / Zig | MIT / Apache-2.0 / MIT | — | Engines only, no renderer/IME; libghostty-vt has no stable API | ✗ for this task |

### 2.2 Linux server side and precedents

| Project | What it is | License [V] | Activity | Conclusion | Suitability |
|---|---|---|---|---|---|
| [bluez/bluer](https://github.com/bluez/bluer) | Rust: BlueZ D-Bus (GATT server, advertising, agent) + L2CAP sockets | BSD-2-Clause | v0.17.4, 2026-08 | AcquireWrite/AcquireNotify as IO (`gatt/local.rs:902-962`), device address and MTU in requests, L2CAP `StreamListener` with `set_security`, MTU | ★★★ base of the daemon |
| [kevincar/bless](https://github.com/kevincar/bless) | Python GATT server | MIT | 2026-04 | Works, has AcquireNotify; depends on dbus-next (not updated since 2024-06) | ★★ for prototypes |
| [ukBaz/python-bluezero](https://github.com/ukBaz/python-bluezero) | Python GATT server | MIT | 2025-03 | No Acquire* | ★ |
| dbus-fast | Python D-Bus | MIT | active | Base for our own Python server | ★★ |
| tinygo-org/bluetooth | Go | BSD-3-like | active | Does not report which device wrote (`gatts_linux.go`) | ✗ for multi-client |
| go-ble/ble, @abandonware/bleno, BTstack | — | — | — | Take over HCI (user channel) / require stopping bluetoothd | ✗ (exclusive access) |
| [radxa/linkr-bee](https://github.com/radxa/linkr-bee) | ESP32 (Zephyr) bridge of UART console → BLE/Wi-Fi + clients | Apache-2.0 | active | Not a Linux server. Useful: reliable protocol with seq/indications, documented data loss on queue overflow (`docs/BRIDGE_QUEUE_FINDINGS.md`), single connection, no resize | ★ reference; "external bridge" option |
| [sorah/bluetooth-getty](https://github.com/sorah/bluetooth-getty) | Classic SPP → per-connection systemd unit with agetty | MIT | — | Closest packaging precedent: .deb, hardened unit, PTY proxy because of `vhangup()` in login (`src/session_proxy.rs`) | ★★ packaging/systemd template |
| [monperrus/asteroid-ble-shell](https://github.com/monperrus/asteroid-ble-shell) | NUS shell on AsteroidOS | GPL-3.0 | 2026-09 | Best security model among the precedents (auth flags, "arm" window, device pinning, SIGHUP→TERM→KILL escalation); PTY 80×24 without resize; no PAM | ★ reference |
| [VishovVladimir/ble_tty](https://github.com/VishovVladimir/ble_tty), [jonasneves/pi-loop](https://github.com/jonasneves/pi-loop) | NUS ↔ root bash on a Pi | no license | 2026 | No authentication or encryption; 20-byte chunks without flow control | ✗ (counterexample) |
| [cocoahuke/coBlue](https://github.com/cocoahuke/coBlue) | BlueZ C daemon + macOS client | MIT | 2018 | The iOS client was never written | ✗ |
| [Jakeler/ble-serial](https://github.com/Jakeler/ble-serial) | Desktop: BLE UART → virtual serial | MIT | 2024-12 | Useful as a test client | ★★ for tests |
| Zephyr/NCS `CONFIG_SHELL_BT_NUS` | MCU shell over NUS | Apache-2.0 | — | The most widespread "shell over BLE", but MCU, not Linux | ★ |
| `rfcomm watch … getty` | Classic Bluetooth console | — | — | iOS cannot use RFCOMM/SPP without MFi | ✗ |

**External UART bridge (HM-10/ESP32 → getty):** no documented "phone → BLE bridge → Linux getty → full-screen applications" project was found [V: absent from search]. A bridge adds a second flow-control boundary and does not pass the window size; it is justified only for hosts without their own BLE (in which case linkr-bee is the best base).

---

## 3. Comparison of BLE transports

### 3.1 Platform constraints

| Parameter | iOS (CoreBluetooth) | Android | Linux/BlueZ (Pi) |
|---|---|---|---|
| ATT MTU | Cannot be requested; the OS chooses itself (185/247/527 observed depending on model/version). Chunk size — `maximumWriteValueLength(.withoutResponse)` [V ADG §58.11; S] | `requestMtu`; since Android 14 always 517 on the first call [V] | `ExchangeMTU = 517` by default [V] |
| Max. attribute value | 512 | 512 (`GATT_MAX_ATTR_LEN`) [V AOSP] | A notification longer than MTU−3 is **silently truncated** (`shared/gatt-server.c:1828`) [V] |
| DLE / 2M PHY | Automatic; the peripheral must do LL Length Update before MTU exchange [V ADG]; PHY — on peripheral's initiative [S] | `setPreferredPhy` (API 26) [V] | CYW43455 controller — to be checked [H] |
| Connection interval | Min. 15 ms, multiple of 15; 11.25 ms only with HID; requested by the peripheral [V ADG §58.6] | `requestConnectionPriority(HIGH)` = 11.25–15 ms [V AOSP config.xml] | Peripheral-side parameter request is not exposed via D-Bus [H] |
| Write flow control (client → server) | `canSendWriteWithoutResponse` + `peripheralIsReady(toSendWriteWithoutResponse:)` [V] | One operation in flight; `onCharacteristicWrite` also arrives for WNR, delayed under congestion [V AOSP `GattService`] | AcquireWrite: a write is acknowledged before the application reads it; WNR is **silently dropped** on EAGAIN (`gatt-database.c:3348-3355`) [V] |
| Notification backpressure (server → client) | No API [V] | No API [V] | The `att->write_queue` queue is unbounded (`shared/att.c:1681-1687`) [V] |
| L2CAP CoC | iOS 11+ `openL2CAPChannel(PSM)`; PSM from the characteristic `ABDD3056-28FA-441D-A470-55A75A52553A` [V]; bug with several channels to one device [S] | API 29+ `createL2capChannel` (requires a bond) / `createInsecureL2capChannel` [V] | `SOCK_SEQPACKET` socket, PSM 0x80–0xFF without privileges (`l2cap_sock.c:69-80`), credit-based FC in the kernel [V] |
| Background | `bluetooth-central`: notifications wake the app (~10 s); state restoration; not relaunched after force-quit/Bluetooth off (QA1962) [V]; CoC in the background is undocumented [H] | FGS `connectedDevice` (Android 14+) [V]; CompanionDeviceManager [V] | — |
| Pairing | Cannot be initiated from the app; trigger — ATT Insufficient Authentication/Encryption [V ADG §58.10] | `createBond()` or implicitly on an authentication error [V] | IO capability is set by the default agent, adapter-wide [V `adapter.c:9508-9535`] |

### 3.2 Transport options

| Criterion | Raw NUS | Custom GATT profile (recommended) | L2CAP CoC |
|---|---|---|---|
| Compatibility with third-party UART apps | Yes (line mode only) | No (optionally — a separate NUS compatibility service) | No |
| Resize/signals/control messages | No (in-band hacks only) | Yes (in-band framing) | Yes |
| Backpressure server → client | No | Application-level credits | Native, kernel credits; on kernels ≥ 6.10 also based on free receive buffer space (commit `ce60b9231b66`) [V] |
| Backpressure client → server | Client OS + risk of loss in bluetoothd | Client OS + server credits | Native |
| Multi-client on BlueZ 5.66 | No (broadcast) | No (single client only) | Yes (socket per `accept()`) |
| Multi-client on BlueZ ≥ 5.69 | Partial | Yes (per-device fd + HUP on disconnect) | Yes |
| Throughput | 30–100 KB/s [S] | 30–100 KB/s [S] | WWDC17: up to ~49 KB/s on iOS (L2CAP+DLE+15 ms); Nordic: gain over GATT is insignificant [S] |
| Confirmed iOS/Android ↔ BlueZ compatibility | Yes | Yes | **No public confirmation found** [S] |
| iOS background operation | Yes | Yes | Undocumented [H] |
| Complexity | Minimal | Small (~1–3-byte frame header + credits) | Medium (+ GATT is still needed for PSM discovery) |

**Conclusion:** the custom GATT service is the baseline and mandatory transport; L2CAP CoC is an optional improvement after experiment E2; raw NUS — only as a separate optional compatibility mode.

For a terminal, throughput is not the bottleneck: a full redraw of a 200×60 screen with SGR is 12–40 KB (0.2–1 s in the worst case), incremental vim/htop updates are much smaller [H]. The decisive factors are **latency** (keystroke echo ≈ 2 connection intervals: ~30–60 ms) and **correct backpressure** during `cat bigfile` [S/H].

---

## 4. Android and iOS client side

### 4.1 What the code of existing BLE UART apps showed

- All of them are a message log, not a terminal: no CSI parser, cell grid, alternate screen, Ctrl/Esc/arrows [V, app. A §1].
- A typical mistake is decoding UTF-8 per notification separately (`new String(data)` in SimpleBluetoothLeTerminal `TerminalFragment.java:254`, `String(it)` in nRF Toolbox Android `UARTManager.kt:44`): multibyte characters on packet boundaries get corrupted. A terminal client must pass **raw bytes** to the engine.
- Useful patterns: write queue driven by `onCharacteristicWrite` (Android), WNR gating by `canSendWriteWithoutResponse` (iOS), Telit TIO credits as a backpressure model [V].
- Conclusion: no UART app is suitable as a base for a fork; terminal/SSH clients are what should be taken.

### 4.2 Terminal engines (from the sources)

| Engine | Bytes in | Bytes out | Resize | xterm level | Unicode width | Input |
|---|---|---|---|---|---|---|
| ConnectBot termlib | `writeInput` (`TerminalEmulator.kt:107`) | `onKeyboardInput` callback (`:252-275`) | `onResize(cols, rows, px)` | truecolor, mouse, alt screen, bracketed paste, OSC 52 | **old Markus Kuhn table** — risk of mismatch with glibc for emoji [H] | TYPE_NULL + compose mode (`ImeInputView.kt`) |
| Termux terminal-emulator | `TerminalEmulator.append` (`:688`) | `TerminalOutput.write` | `TerminalSession.updateSize` → JNI (needs a patch) | same + sixel | Unicode 15 | best IME |
| SwiftTerm | `feed(byteArray:)` (`AppleTerminalView.swift:1264`) | `send(source:data:)` (`TerminalViewDelegate.swift:73`) | `sizeChanged` (`:57`) | same + Kitty graphics/keyboard; esctest | **Unicode 17** | `UITextInput`, accessory bar with Esc/Ctrl (`iOSAccessoryView.swift:22`) |

All three take and emit **raw bytes** and report the size as (cols, rows) — this maps directly both onto our own protocol and onto SSH `window-change`.

### 4.3 Ready-made "engine + SSH" combinations

- **Android — ConnectBot** (Apache-2.0) already ties termlib and sshlib together (`service/TerminalBridge.kt:490-530`: `onKeyboardInput`/`onResize` → `WriteData`/`SetDimensions`). sshlib accepts an arbitrary transport via `SshClientConfig.transportFactory`, and **ConnectBot already uses this** for jump-host chains: `transport/SSH.kt:976-1011` (connectbot `0bea3773`) [V, checked manually]. So the BLE transport for SSH in ConnectBot is an implementation of `org.connectbot.sshlib.transport.Transport` over GATT + a new host type (`ble://…`) in the UI. Along the way, for free: host manager, keys, extra-keys (`ui/components/TerminalKeyboard.kt`, Apache-2.0), background service, notifications.
- **iOS — SwiftTerm** contains an SSH terminal demo on swift-nio-ssh (`TerminalApp/iOSTerminal/UIKitSshTerminalView.swift`, SwiftTerm `c3436be3`): `send` → SSH channel, `sizeChanged` → window-change, channel data → `feed`. For BLE, the NIO TCP bootstrap has to be replaced with a channel over CoreBluetooth [H: moderate effort; SwiftNIO SSH in its own tests runs over the in-memory `EmbeddedChannel` — `EndToEndTests.swift:26-44`]. There is no ready iOS app to fork: Blink is GPLv3, La Terminal / Secure ShellFish are closed. The iOS client is a new app built from mature components.

### 4.4 Code shared between platforms

Examined: Flutter (xterm.dart + universal_ble), WebView + xterm.js with a native BLE bridge, libvterm/alacritty_terminal via FFI, KMP. Conclusion [H, app. B §4]: **native UI on each platform**; shared — only the BLE framing specification with test vectors (≈500 lines of implementation per platform) or, optionally, Kable/KMP for the BLE layer. A shared terminal engine does not pay off: the renderer, IME, text selection and accessibility are platform-specific anyway, and mobile input in xterm.js is problematic (open issues).

### 4.5 Background and reconnection

- iOS: the `bluetooth-central` mode keeps the GATT connection, notifications wake the app; `connect()` has no timeout and is suitable for auto-reconnect; iOS 17 — `EnableAutoReconnect`. After a force-quit or Bluetooth being turned off, the system does not relaunch the app (QA1962) [V].
- Android: an open session — only in a foreground service of type `connectedDevice`; for background reconnection — `autoConnect=true` or CompanionDeviceManager presence [V].
- Since per the brief the session ends when BLE disconnects, "reconnection" = a new connection + a new login. The session persistence policy (tmux on the server or at the daemon level) is a separate decision (question 6). The brief defers the detailed background policy.

---

## 5. Linux/BlueZ server side

### 5.1 Interaction with BlueZ

- **BlueZ D-Bus API only** (`GattManager1.RegisterApplication`, `LEAdvertisingManager1.RegisterAdvertisement`) — no exclusive access to the adapter; registrations are bound to the D-Bus connection and disappear when the process stops (`src/advertising.c:225`) — convenient for a removable package [V].
- **D-Bus policy** upstream and in Trixie allows any local user to call `org.bluez` methods (`src/bluetooth.conf:26-28`); in Bookworm a Debian patch additionally allows the `bluetooth` group [V]. The daemon can run as a separate unprivileged user (a member of the `bluetooth` group if needed). Consequence: bluetoothd is not a security boundary between local users — the authentication boundary is SSH/PAM.
- **`--experimental`, `--compat` and editing `main.conf` are not needed** for the GATT server, Acquire*, advertising, or L2CAP [V, app. C §1.8].
- **AcquireWrite/AcquireNotify** (fd path, `doc/org.bluez.GattCharacteristic.rst:156-246`) are preferable to `WriteValue`/`PropertiesChanged`: less D-Bus traffic, MTU arrives in the options; the socket must be created as `SOCK_SEQPACKET|SOCK_NONBLOCK` (as in `client/gatt.c:2568` and bluer). bluer subtracts 5 bytes from the MTU because of data truncation in BlueZ (`bluer/src/gatt/mod.rs:358-364`) [V].
- **BlueZ versions** [V]: Debian/RPi OS Bookworm — 5.66, Trixie — 5.82. Per-device AcquireWrite/AcquireNotify (separate fds per client, HUP on disconnect) appeared in commit `8eb1dee87e01` (5.69). On 5.66: one fd per characteristic, notifications are sent to all subscribers, writes from different devices are merged; disconnection has to be tracked via `Device1.Connected`.
- **Advertising** stops on an incoming connection and is automatically resumed by the kernel only after a disconnect (`hci_event.c:5893`, `:3574`); for a second client the daemon must re-enable advertising; support for this state by the CYW43455 controller has not been verified [V/H].
- **Coexistence:** the GATT service and advertising are added alongside other services; an L2CAP listener on a dynamic PSM does not conflict with bluetoothd [V]. Only pairing settings (IO capability, Pairable/Bondable) affect the whole adapter — which is why the recommended scheme does not require changing them (section 6.5).

### 5.2 Flow control on the server

- An **application-level credit protocol** is needed in both directions [V rationale: unbounded notification queue; WNR loss in AcquireWrite on EAGAIN].
- Server → client direction: the client grants credits in bytes as it consumes data; the daemon does not read from the session (stdout of `sshd -i` or the PTY master) when credits run out — the writer then blocks normally (PTY kernel / socket). This gives natural backpressure up to `cat bigfile`.
- Client → server direction: clients are limited by their OSes (one write in flight / `canSendWriteWithoutResponse`), but so that bluetoothd does not lose WNR when the daemon is slow, the daemon grants the client a receive window (credits) matching the free space in its buffer.
- A mechanism with retransmission and sequence numbers (as in linkr-bee) is **not needed**: the BLE link layer is reliable (ACK/CRC), losses occur only in software queues, which are covered by credits; and on disconnect the session ends anyway [H, justified].

### 5.3 Session: PTY, PAM, controlling TTY, termination

Two options, common to both architectures:

**(a) `sshd -i` (recommended)** — the daemon passes the stream to a per-connection `sshd -i`:
- `sshd -i` (inetd mode) is supported, including after the sshd/sshd-session split (`sshd.c:1400-1402, 1865-1870`); the OpenSSH regression suite itself runs the server via `ProxyCommand … sshd -i` (`regress/test-exec.sh:887`) [V].
- sshd itself does PAM (`UsePAM yes`), allocates the PTY, `setsid`/`TIOCSCTTY`, utmp/wtmp/lastlog, `window-change` → `TIOCSWINSZ`, SIGHUP on close; `pam_systemd` creates a logind session [V for the mechanisms, app. C §5].
- A separate config (`sshd -i -f /etc/ble-term/sshd_config`) — does not affect the network sshd, works even if `ssh.service` is disabled. In Debian Bookworm (openssh-server 9.2) `ssh.socket` is `Accept=no`, there is no `ssh@.service` template [V, locally] — so the package installs its own template.
- Privileges: launch via systemd socket activation — `ble-term-session.socket` (`ListenStream=/run/ble-term/session.sock`, `Accept=yes`, `SocketUser=_bleterm`, `SocketMode=0600`) + `ble-term-session@.service` (`ExecStart=-/usr/sbin/sshd -i -f …`, `StandardInput=socket`). **Our privileged code is zero lines**; the daemon that parses radio input runs without root [H, standard systemd technique].
- Termination: BLE disconnect → the daemon closes the unix socket → sshd gets EOF → closes the channel and PTY → SIGHUP to the session process group, `pam_close_session`; the unit stops, its cgroup is killed [V mechanism; H for the details in combination].

**(b) `login`/`agetty` on a PTY (literal-brief option):**
- The same socket-activated root unit, but it starts a small PTY helper: `openpty` → `setsid` → `TIOCSCTTY` → `login -p -h ble:<addr>` (like shellinabox) or `agetty -` [V, app. C §5.1]; the master fd is passed to the daemon via `SCM_RIGHTS`, or the helper proxies by itself.
- Must be taken into account: util-linux `login` does `vhangup()` (`login.c:600-622`), the master temporarily returns EIO — the proxy must survive this (like `systemd/src/shared/ptyfwd.c:905-912` and bluetooth-getty) [V].
- Resize — via our own control message → `TIOCSWINSZ` on the master.
- This helper is our own root code (~200–400 lines, C or Rust) [H].

**For both options:** `pam_systemd` moves the user's processes into `session-N.scope`, so `KillMode=control-group` of our unit **does not kill** the shell tree; the default behavior is like ssh (SIGHUP; tmux/nohup survive). For a strict "kill everything" — `org.freedesktop.login1.Manager.TerminateSession` [V mechanism; question 6].

### 5.4 Libraries and daemon language

Recommended: **Rust + bluer** (BSD-2): the only library found that simultaneously has a GATT server via D-Bus with AcquireWrite/AcquireNotify, device information and MTU in requests, L2CAP sockets with a security level, and async; the output is a static binary with minimal dependencies [V, app. C §3]. Alternative — Python + dbus-fast (faster prototype, more runtime dependencies; bless is tied to the unmaintained dbus-next).

### 5.5 Package

`.deb` (daemon binary; `ble-term.service` as `_bleterm`/`DynamicUser`, `After=bluetooth.service`, `PartOf=bluetooth.service` for re-registration when bluetoothd restarts; `ble-term-session.socket` + `ble-term-session@.service`; `sysusers.d`; `/etc/ble-term/sshd_config` and the daemon config). Dependencies: `bluez`, `openssh-server` (sshd binary only; the network service can stay disabled). Removal: stopping the units — the registrations in BlueZ disappear; created bonds (if pairing is enabled) remain in `/var/lib/bluetooth` — to be documented. Hardened unit template — sorah/bluetooth-getty (`RestrictAddressFamilies=AF_UNIX AF_BLUETOOTH`, `ProtectSystem=strict`, etc.) [V].

---

## 6. Proposed architecture

### 6.1 Components

```
 ┌──────────────── Android ────────────────┐      ┌────────────────── iOS ──────────────────┐
 │ ConnectBot (fork, Apache-2.0)            │      │ New application                          │
 │  termlib (libvterm) ── TerminalBridge    │      │  SwiftTerm TerminalView                  │
 │        │ bytes / onResize                │      │        │ send / sizeChanged / feed        │
 │  sshlib SshClient (host-key pinning,     │      │  SwiftNIO SSH (NIOSSHHandler,             │
 │        pubkey/password, window-change)   │      │        host-key pinning)                 │
 │        │ Transport (read/write/close)    │      │        │ custom NIO Channel               │
 │  BleTransport ── BleStream (credits,     │      │  BleStream (credits, frames)             │
 │        frames) ── android.bluetooth GATT │      │        └── CoreBluetooth GATT            │
 │  FGS connectedDevice, CDM                │      │  bluetooth-central background mode       │
 └────────────────────┬─────────────────────┘      └────────────────────┬─────────────────────┘
                      │            BLE (LE Secure Connections — optional)                    │
                      └───────────────┬────────────────────────────────────┘
                                      │  GATT service "BLE-Term"
                                      │   • Stream-In  : write-without-response (+write)
                                      │   • Stream-Out : notify
                                      │   • Info       : read (protocol version, capabilities)
                                      │   • L2CAP PSM  : read, UUID ABDD3056-… (stage 4)
 ┌────────────────────────────────────┼─────────────── Linux (Raspberry Pi) ─────────────────────┐
 │  bluetoothd (system, no configuration changes)                                                │
 │        ▲ D-Bus: GattManager1, LEAdvertisingManager1, AcquireWrite/AcquireNotify (fd)           │
 │  ble-term-daemon (Rust/bluer, user _bleterm, no root)                                          │
 │     • advertising, GATT application, connection tracking (Device1.Connected / HUP fd)          │
 │     • BleStream: framing, credits, timeouts, client limit                                      │
 │     • per connection: connect() to /run/ble-term/session.sock                                  │
 │        ▼ AF_UNIX (SSH byte stream)                                                             │
 │  systemd: ble-term-session.socket (Accept=yes) → ble-term-session@N.service (root)             │
 │        ExecStart=-/usr/sbin/sshd -i -f /etc/ble-term/sshd_config   StandardInput=socket        │
 │           └─ PAM (common-auth, pam_systemd, pam_faillock) → PTY /dev/pts/N → login shell        │
 └───────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 Data flows

```
Keystroke (Android):
 termlib.onKeyboardInput(bytes) → SshClient channel data → sshlib encrypts (≈36–48 B)
 → BleTransport.write → BleStream: DATA frame, waits for a credit → writeCharacteristic(WNR)
 → onCharacteristicWrite → next frame
 → bluetoothd → AcquireWrite fd → daemon: unwrap frame, data → unix socket → sshd stdin
 → PTY master → line discipline → application (vim)

Output (server → client):
 vim → PTY → sshd → unix socket → daemon reads ONLY if the client has credits
 → frames ≤ MTU−5 → AcquireNotify fd → bluetoothd → notify
 → client: BleStream reassembles the stream, sends CREDIT as the SSH layer consumes it
 → sshlib/SwiftNIO SSH decrypts → termlib.writeInput / SwiftTerm.feed

Resize:
 termlib.onResize / SwiftTerm.sizeChanged → SSH "window-change" → sshd → TIOCSWINSZ → SIGWINCH

BLE disconnect:
 HUP on per-device fd (BlueZ ≥5.69) or Device1.Connected=false (5.66) → daemon closes
 unix socket → sshd: EOF → close PTY → SIGHUP to session → pam_close_session → unit finished
```

### 6.3 BleStream protocol (draft, to be refined after experiments)

- One GATT service with its own 128-bit UUID; two stream characteristics. Control messages are **in-band** (order relative to data is preserved, independent of how the OS schedules different characteristics).
- Frame = 1 type byte + data: `HELLO(version, caps, rx_window)`, `DATA(bytes)`, `CREDIT(u32 bytes)`, `CLOSE(reason)`, `PING/PONG`. No sequence numbers (rationale in 5.2).
- Before the first `DATA` — a `HELLO` exchange; the server does not open a session (does not start `sshd`) before `HELLO` — protection against "empty" connections; timeout of an idle/not-yet-authenticated connection → `Device1.Disconnect()`.
- Frame size — based on the MTU received in the AcquireNotify/AcquireWrite options, minus the bluer margin.
- The transport is opaque: inside is the SSH stream (option 1) or a terminal stream with `RESIZE` (option 7.1). This allows choosing the session option (question 1) without changing the BLE layer.

### 6.4 Multi-client

MVP — **one active client** (others get `CLOSE(busy)` and are disconnected). The architecture (per-connection state, a separate `sshd -i` per connection) allows multi-client: on BlueZ ≥ 5.69 via per-device fds, on 5.66 — only via L2CAP CoC. In addition, experiment E3 is needed (advertising while a connection is active on CYW43455).

### 6.5 Security (recommended scheme)

- **Server authenticity:** an SSH host key, dedicated to BLE-Term or the system one; the client pins the fingerprint via a QR code/printed fingerprint or TOFU on first connection. This closes MITM on first contact and phishing via a fake "Pi" (BLESA class), which BLE pairing on a headless device does not provide [V, app. E §1, §3].
- **User authenticity:** standard PAM via sshd (password) and/or a public key from Android Keystore / iOS Secure Enclave; `pam_faillock`, `MaxAuthTries`, root login disabled in `/etc/ble-term/sshd_config`.
- **Confidentiality:** SSH encryption (curve25519 + AES-GCM/ChaCha20-Poly1305); disable `ObscureKeystrokeTiming` (sshlib: `obscureKeystrokeTimingIntervalMs = 0`), otherwise ~2 KB/s of overhead traffic [V].
- **BLE level by default — no mandatory pairing** (encrypt flags are not set): no default agent is needed, the adapter's Pairable/Bondable/IO capability are not changed, there are no system pairing dialogs and no problems with "forgotten" bonds on iOS. An attack on the channel before SSH authentication is equivalent to an attack on a network sshd — a standard, well-protected surface.
- **Optional hardening (config):** LE Secure Connections Just Works + a default agent with a "pairing window" opened by button/CLI (`RequestAuthorization`, `device.c:7972`) + `encrypt-*` flags — reduces the DoS/pre-authentication surface and hides traffic metadata [V mechanism]. A static passkey on Linux is **impossible** without a kernel patch (the key is generated by the kernel, `smp.c:2805-2815`) and would be weak anyway (CVE-2020-26558) [V].
- Regardless of mode: "proximity" is not an authentication factor (NCC 2022 relay attacks), auto-login by a bonded device is forbidden [V].

### 6.6 Brief criteria

| Requirement | How it is met |
|---|---|
| BLE only, no IP | GATT/L2CAP; SSH is just a protocol over a byte stream, there is no network |
| vim/htop/tmux/nmtui, xterm, Unicode, Ctrl, resize | termlib/SwiftTerm + SSH window-change |
| Reuse of a mature client | ConnectBot fork; SwiftTerm + SwiftNIO SSH (ready combination in the SwiftTerm demo) |
| Standard PTY and PAM, system authentication | sshd: PTY + PAM + logind; `login`/`agetty` — only in option 7.1 (question 1) |
| Minimal intrusion into BlueZ, coexistence | D-Bus registrations only, no `main.conf` edits, no adapter takeover, no default agent by default |
| Install/remove as a package | `.deb` + systemd units, no persistent state in bluetoothd |
| Session termination on disconnect | EOF → sshd → SIGHUP; strict mode — TerminateSession |
| Least privilege | Daemon without root; root only for sshd via socket activation |
| No root/jailbreak on phones | Public APIs only (CoreBluetooth, android.bluetooth) |

---

## 7. Alternative architectures and reasons for rejection

### 7.1 Custom terminal protocol + `login`/PAM (main alternative — not finally rejected)

BleStream with `DATA`/`RESIZE`/`SIGNAL` frames; on the server, a PTY helper with `login -p -h ble:<addr>`; clients connect termlib/SwiftTerm to BLE directly (glue ~0.5–1 day per platform [H]).

- **Pros:** literally matches the brief (`agetty`/`login`); no SSH libraries in the clients; ~35–50 bytes less per keystroke; `openssh-server` not needed.
- **Cons:** (1) security rests entirely on BLE pairing, and on a headless device it is either Just Works (no MITM protection, the login password travels in plaintext inside the channel) or requires displaying a passkey — impractical without a display [V, app. E §3]; adding protection at the application level (Noise) = reinventing SSH; (2) our own root helper (PTY, vhangup/EIO, utmp, passing the master fd); (3) on Android the ready ConnectBot–SSH integration is lost — our own "host type" and transport for termlib are needed; (4) no key-based authentication.
- **Status:** fallback option; the BLE layer is designed so that switching between 6 and 7.1 does not change the transport.

### 7.2 Raw NUS + PTY (pi-loop, ble_tty, asteroid-ble-shell)
No resize, no backpressure, no control channel; compatible with third-party UART apps, but only in line mode. **Rejected** as the main option; possible as a separate compatibility mode disabled by default (question 7).

### 7.3 L2CAP CoC as the only transport
Best backpressure and multi-client on any BlueZ version, but no confirmed iOS/Android ↔ BlueZ compatibility, iOS background operation is undocumented, iOS bugs with several channels, Android fragmentation; GATT is needed for PSM discovery anyway. **Rejected as the only transport**; planned as an improvement (stage 4) after experiment E2.

### 7.4 Noise (or another application-level channel) + custom protocol
Lighter than SSH (handshake ~100–200 B), but user authentication, PTY/resize, rekeying, and auditing would all have to be redone; the libraries are less proven. **Rejected** — SSH provides all of this with mature code, and its overhead for BLE is acceptable (handshake ~3–8 KB, ~0.1–0.5 s [H]).

### 7.5 Shared cross-platform terminal UI (Flutter / WebView+xterm.js / libvterm-FFI / KMP)
xterm.dart has slowed down and works with strings; flutter_blue_plus is a licensing trap; xterm.js has mobile input problems; FFI engines need their own renderers (6–10+ person-weeks). **Rejected** in favor of native mature engines [H estimates, app. B §4].

### 7.6 External BLE-UART bridge (ESP32/linkr-bee → serial getty)
Requires neither BLE on the host nor Linux changes, but: additional hardware, no resize, a second flow-control boundary, linkr-bee — single connection and documented queue losses. **Rejected** for the reference platform (the Pi has its own BLE); appropriate as a separate product for hosts without BLE.

### 7.7 Stacks with exclusive HCI access (bleno, go-ble, BTstack)
Contradict the coexistence requirement. **Rejected.**

### 7.8 Bluetooth Classic (RFCOMM/SPP getty)
Not available to iOS apps without MFi [V, MFi FAQ]. **Rejected.**

### 7.9 Termux / Blink fork
Termux-app is GPLv3 (only the Apache-licensed engine libraries are used); Blink is GPLv3, distributing a fork via the App Store is legally risky [H]. **Rejected** as a base for the apps.

---

## 8. Risks, unknowns and required experiments

| # | Risk / unknown | Impact | Experiment |
|---|---|---|---|
| E1 | Real MTU, connection interval, echo latency and GATT notify/WNR throughput between the Pi (CYW43455) and current iPhones/Android phones; bluetoothd behavior under a notification burst | Responsiveness, credit window size | BleStream prototype on bluer: measure echo RTT, KB/s, bluetoothd memory during `cat` with and without credits |
| E2 | iOS/Android ↔ BlueZ L2CAP CoC (without and with encryption), whether iOS initiates pairing on a security rejection, CoC in the background on iOS | Stage 4 (CoC) | `openL2CAPChannel` / `createInsecureL2capChannel` to a bluer `StreamListener` |
| E3 | Several simultaneous LE connections on CYW43455; re-enabling advertising while a connection is active | Multi-client | `btmgmt info`, LE supported states, two phones |
| E4 | sshlib over a BLE `Transport` on Android (JVM 17 target, desugaring); SwiftNIO SSH over a custom Channel on CoreBluetooth; handshake time (mlkem768x25519 vs curve25519) | Client timelines | Minimal PoCs on both platforms |
| E5 | Unicode width mismatch (old libvterm table in termlib vs glibc on the Pi) | vim/htop corruption with emoji/CJK | Emoji/CJK test strings in vim/htop/tmux; if needed, update the table in termlib or switch to the Termux engine |
| E6 | iOS background behavior with an open session (suspension, wake-up frequency), Android FGS on OEMs (Samsung/Xiaomi) | Background UX | Screen lock/app switching scenarios |
| E7 | `sshd -i` via socket activation: termination on EOF, utmp/`who`, `pam_systemd`, behavior on OpenSSH 9.2 (Bookworm) and 10.x (Trixie) | Session correctness | Test on both OS releases |
| E8 | BlueZ 5.66: no HUP on the fd on disconnect, shared fd; "client vanished without disconnect" scenarios | Hung sessions | Test with BT turned off on the phone, moving out of range; supervision timeout |
| E9 | The battery plugin in bluetoothd causes repeated pairing requests from iOS when connecting to a Linux peripheral (Ditto report) [S] | UX without pairing | Check on 5.66 and 5.82; if confirmed — find a workaround without editing system configuration, or document it |
| E10 | Peripheral-side connection parameter request (15 ms) and 2M PHY without editing BlueZ configuration | Latency on iOS | Check what iOS picks by default; if needed — `hcitool`/mgmt equivalents (without changing `main.conf`) |

Other risks: termlib is pre-1.0 (0.3.x), the API may change; termux terminal-emulator — the Apache license is stated only in the root `LICENSE.md`; App Store review — the `bluetooth-central` background mode has to be justified.

---

## 9. MVP and development stages

| Stage | Content | Result |
|---|---|---|
| 0. Experiments | E1, E4 (minimal PoCs), E7, E9; if resources allow — E2 | Confirmed credit window and frame parameters, CoC/non-CoC choice |
| 1. Server + desktop client (MVP) | `ble-term-daemon` (Rust/bluer): GATT service, advertising, BleStream with credits, single client, socket-activated `sshd -i`; desktop `ble-term-connect` (Rust/bluer as central) for `ssh -o ProxyCommand="ble-term-connect <addr>" user@pi`; `.deb` | Full terminal (vim/htop/tmux/nmtui) over BLE from a laptop; verification of the entire server pipeline |
| 2. Android | ConnectBot fork: `BleTransport` for sshlib, BLE host type (scanning, CDM), FGS `connectedDevice`, host key pinning (QR/TOFU) | ConnectBot-level Android client |
| 3. iOS | App on SwiftTerm + SwiftNIO SSH + CoreBluetooth BleStream, accessory bar, device list, key pinning, Keychain/Secure Enclave | iOS client |
| 4. Further development | L2CAP CoC with fallback to GATT; multi-client (BlueZ ≥ 5.69 / CoC); optional BLE pairing with a button-opened window; QR provisioning; background reconnection; (optional) NUS compatibility mode; option 7.1 if chosen | Complete product |

Option 7.1 (login/PAM without SSH), if the operator chooses it, changes stages 1–3 as follows: stage 1 adds a root PTY helper and the `RESIZE` frame; in stages 2–3 termlib/SwiftTerm connect to BleStream directly (without SSH libraries), and the Android client is our own app on termlib or a stripped-down ConnectBot fork.

---

## 10. Decisions requiring operator input

1. Session layer: SSH over BLE (`sshd -i`) or our own protocol with `login`/`agetty` (deviation from the letter of the brief vs security and reuse).
2. BLE channel security level: no mandatory pairing (protection via SSH) or mandatory LE SC pairing with a button-opened window/passkey; how the host key is trusted (QR / TOFU).
3. Target OS/BlueZ versions: whether to support Bookworm (BlueZ 5.66) on par with Trixie.
4. Android client: ConnectBot fork or our own app on its libraries.
5. Server daemon language: Rust (bluer) or Python.
6. Session termination policy on disconnect: SIGHUP "like ssh" (tmux survives) or forced termination of the logind session.
7. Whether a compatibility mode with NUS/third-party UART apps is needed.
8. Multi-client: whether it is required in the first version.

Detailed formulations with recommendations — in the message to the operator upon completion of stage 1.
