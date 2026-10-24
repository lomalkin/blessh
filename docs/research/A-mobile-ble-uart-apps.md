> Research working notes (2026-10-24), appendix to `ble-terminal-research.md`. Paths like `scratchpad/repos/...` refer to temporary clones; line numbers match the SHAs given.

# A. Mobile BLE UART / BLE terminal apps — survey (2026-10-24)

Scope: open-source Android/iOS apps and libraries that move a byte stream over BLE (mostly Nordic UART Service, NUS), and real "shell over BLE" projects. The question: what can a Termux-grade terminal client over BLE to a Linux/BlueZ server reuse?

Legend: **[V]** = verified in source or metadata (file:line or URL cited). **[H]** = hypothesis or inference, not verified.

Repos cloned (`--depth 1`) to `/tmp/claude-1000/-home-user--projects-ble-term/44e98b07-3485-4b11-a71e-3f64c9e95675/scratchpad/repos/`.

| Repo (local dir) | Commit SHA | Commit date |
|---|---|---|
| kai-morich/SimpleBluetoothLeTerminal | f322c06c84c60a1e647048094c0e58e683a2a1ea | 2026-04-12 |
| nordicsemi/Android-nRF-Toolbox | c7f8e443ab706fe5a2c03368a05488880c8299ed | 2026-08-26 |
| nordicsemi/IOS-nRF-Toolbox | d419fb24df9f4078ed36e21a55f5b7d21fff8839 | 2026-10-22 |
| nordicsemi/Kotlin-BLE-Library | 4fe3006f320588f3b12b4b79b83cfa94472d9824 | — |
| nordicsemi/Android-BLE-Library | da698f297c75784ca3932bc19777cde93ec7c975 | 2026-10-10 |
| adafruit/Bluefruit_LE_Connect_v2 (iOS) | 26da88b2a667e1de42e029959f405f40130a10d8 | 2024-02-29 |
| adafruit/Bluefruit_LE_Connect_Android_V2 | f2fadabee9b05df1f62c9c97354813d1f8de943a | 2026-02-16 |
| JuulLabs/kable | ad88767b67ae60990b63cd36897b38e7d69aef61 | 2026-10-21 |
| jonasneves/pi-loop | daf00c757652a8e321825d63425381e17d516276 | 2026-10-21 |
| monperrus/asteroid-ble-shell | 36d19c6ad8fba00007d4c974f2b1017ce3965f8d | 2026-09-03 |
| cocoahuke/coBlue | 3f2a178c0240f3aeb966dadd463076b441acc70a | 2018-01-28 |
| hannesweisbach/ble-tty | 31951236713c1c3c3782f445620b26982d0f2a60 | 2021-03-15 |
| johnhowe/BlueTerm | b5238b100e25714241881f0abbc7f5b0bcf3a81b | 2014-05-07 |
| treehouses/flutter-ble-terminal | e04722f926c7f8b2ae461bd9678c4df0f8251342 | 2022-12 |

GitHub URL pattern for citations: `https://github.com/<owner>/<repo>/blob/<SHA>/<path>#L<n>`.

---

## 1. Key conclusions

1. **[V] No open-source mobile app we found does real VT/xterm emulation over BLE.** Every BLE UART app we read (SimpleBluetoothLeTerminal, nRF Toolbox Android and iOS, Bluefruit LE Connect, treehouses flutter-ble-terminal) renders a *line or message log* (TextView, LazyColumn, ListView, chat bubbles) and sends from a *line-input text field*. None of them parses CSI, has a cell grid, an alternate screen, raw keystroke input, or Ctrl/Esc/arrow keys. vim, htop, tmux and nmtui cannot work in any of them. Kai Morich's closed *Serial Bluetooth Terminal* is also described as a "line-oriented terminal" in its store listing.
2. **[V] The only phone-side terminal emulator over Bluetooth we found is BlueTerm (2014).** It uses Bluetooth Classic SPP/RFCOMM, not BLE, and bundles an old copy of the jackpal Android-Terminal-Emulator VT100 code (`BlueTerm.java:1253 class TerminalEmulator`). It shows the intended architecture (emulator plus byte transport), but it is dead and its code should not be reused.
3. **[V] Real "Linux shell over BLE NUS" servers exist, but their clients are not mobile.**
   - *pi-loop*: Pi, PTY, bash, NUS. Client is a Web Bluetooth browser page.
   - *asteroid-ble-shell*: watch daemon, PTY, `sh`, NUS. Client is a Linux Python/Bleak raw terminal.
   - *coBlue* (2018): RPi BlueZ custom service. Client is macOS only; the author never wrote the iOS client.
   - *ble-tty*: PTY to a BLE peripheral, Linux side.

   None of them has a resize channel. asteroid-ble-shell says so explicitly ("PTY starts at 80×24 ... no terminal-resize control channel"). pi-loop has no authentication (root shell). asteroid-ble-shell has the best security model (see §4).
4. **The new client has to be built from parts:**
   - BLE transport: Kotlin on Android, Swift on iOS, or the KMP library Kable for both.
   - Terminal emulator: Termux `terminal-emulator` + `terminal-view` on Android (Apache-2.0, jackpal-derived; termux-app as a whole is GPLv3). SwiftTerm on iOS (MIT).
   - Our own framing/control protocol (resize, session, maybe credits).

   No BLE UART app is a sensible fork base for the terminal UI. They are useful only as *reference code* for the GATT plumbing.
5. **[V] Patterns worth copying, and anti-patterns found:**
   - **Good, Android:** a serialized write queue driven by `onCharacteristicWrite`, with chunking to `MTU-3` (SimpleBluetoothLeTerminal `SerialSocket.java:349-421`). Kable's Android `write` also awaits `OnCharacteristicWrite` for both write types (`BluetoothDeviceAndroidPeripheral.kt:195-211`).
   - **Good, iOS:** Kable gates write-without-response on `canSendWriteWithoutResponse` / `peripheralIsReady(toSendWriteWithoutResponse:)` under a mutex (`CBPeripheralCoreBluetoothPeripheral.kt:217-229`, `PeripheralDelegate.kt:241-247`). nRF Toolbox iOS does the same in its Throughput screen only (`ThroughputViewModel.swift:176`).
   - **Credit-based flow control:** SimpleBluetoothLeTerminal's `TelitDelegate` implements Telit TIO 2.0 credits (`SerialSocket.java:506-645`). This is a ready model for app-level backpressure in both directions.
   - **Anti-pattern, Bluefruit iOS:** fires `.withoutResponse` writes in a tight loop with no `canSendWriteWithoutResponse` check (`BlePeripheral.swift:506-513`). CoreBluetooth drops writes when its queue is full, so data can be lost. Bluefruit's workaround is to send "one `.withResponse` every N packets" (`BlePeripheral+Uart.swift:215`) and, on Android, a fixed 40 ms delay (`BlePeripheralUart.java:45,255`).
   - **Anti-pattern, everyone:** UTF-8 is decoded per notification. A multi-byte character split across two notifications gets garbled: `new String(data)` in SimpleBluetoothLeTerminal `TerminalFragment.java:254`, `String(it)` in nRF Toolbox Android `UARTManager.kt:44`. nRF Toolbox iOS *drops the whole notification* if it is not valid UTF-8 and trims whitespace (`UARTViewModel.swift:172-175`), which destroys terminal output. A terminal client must feed raw bytes into the emulator's own UTF-8 decoder.
   - **Anti-pattern, server side:** pi-loop sends fixed 20-byte notifications with no backpressure (`bridge/bridge.py:20,56-63`).

---

## 2. Comparison table

| Project | Platform / lang | License (verified) | Stars / last push | Transport details | Flow control | Rendering / input | Reuse / fork suitability |
|---|---|---|---|---|---|---|---|
| [kai-morich/SimpleBluetoothLeTerminal](https://github.com/kai-morich/SimpleBluetoothLeTerminal) | Android, Java | MIT (LICENSE) | 345 / 2026-04-12 | NUS, HM-10 FFE0/FFE1, Microchip, Telit TIO; `requestMtu(512)`; payload = mtu-3 | Own queue + `onCharacteristicWrite`; Telit credits | TextView log, line input, hex mode, `^X` caret display of control chars | **Best Android reference** for GATT state machine and queue. Uses deprecated pre-API-33 `setValue`/`getValue` calls. Not a terminal UI. |
| Serial Bluetooth Terminal (Kai Morich, closed) | Android | proprietary | Play store, v1.5x 2026 | Classic and BLE; v1.48 added nRF write-without-response | n/a | "line-oriented terminal", macros, hex | Not reusable (closed). Popular with makers. |
| [nordicsemi/Android-nRF-Toolbox](https://github.com/nordicsemi/Android-nRF-Toolbox) | Android, Kotlin/Compose | BSD-3-Clause | 1230 / 2026-10-20 | NUS via Kotlin-BLE-Library 2.x; `automaticallyRequestHighestValueLength=true` | Suspending `write()` per chunk (library serializes) | LazyColumn chat log; macros | Good modern Kotlin reference. UI not terminal. |
| [nordicsemi/Kotlin-BLE-Library](https://github.com/nordicsemi/Kotlin-BLE-Library) | Android, Kotlin | BSD-3-Clause | 532 / 2026-10-23 | `requestMtu` 23..517, `maximumWriteValueLength(type)`, GATT **server** module too | suspend/Flow | — | Candidate Android transport library. Its successor-to note says v2 is "early development". |
| [nordicsemi/Android-BLE-Library](https://github.com/nordicsemi/Android-BLE-Library) | Android, Java | BSD-3-Clause | 2425 / 2026-10-13 | `WriteRequest.split()` with `DefaultMtuSplitter`; request queue | onCharacteristicWrite-driven queue | — | Mature, widely deployed. README points to the Kotlin v2 replacement. |
| [nordicsemi/IOS-nRF-Toolbox](https://github.com/nordicsemi/IOS-nRF-Toolbox) | iOS, Swift/SwiftUI | BSD-3-Clause | 668 / 2026-10-23 | NUS; UART uses `writeValueWithResponse` of whole message, no chunking | Per write with response; WwR gating only in Throughput | Message bubbles; trims whitespace; drops non-UTF-8 packets | Reference for nordicsemi/IOS-BLE-Library (Combine) usage. Not a terminal. |
| [nordicsemi/IOS-CoreBluetooth-Mock](https://github.com/nordicsemi/IOS-CoreBluetooth-Mock) | iOS, Swift | BSD-3-Clause | 287 / 2026-10-19 | Drop-in mock of CoreBluetooth | — | — | **Useful for tests/simulator** of an iOS client (used by nRF Toolbox iOS). |
| [adafruit/Bluefruit_LE_Connect_v2](https://github.com/adafruit/Bluefruit_LE_Connect_v2) | iOS, Swift/UIKit | MIT | 177 / 2024-02-29 | NUS; chunks at `maximumWriteValueLength(.withoutResponse)`; own command queue | **None** for WwR (tight loop); optional "withResponse every N" | UITextView log, timestamps, hex | Reference only. Stale (2024). |
| [adafruit/Bluefruit_LE_Connect_Android_V2](https://github.com/adafruit/Bluefruit_LE_Connect_Android_V2) | Android, Java | MIT | 101 / **archived** | `requestMtu(517)`, 2M PHY; command queue | onCharacteristicWrite queue; 40 ms inter-packet delay | Log | Archived; V4 repo exists (2 stars). |
| [JuulLabs/kable](https://github.com/JuulLabs/kable) | **KMP**: Android, Apple, JS, JVM (btleplug) | Apache-2.0 (LICENSE.txt) | 1202 / 2026-10-23 | `requestMtu` (Android), `maximumWriteValueLengthForType` | **Correct on both**: Android awaits OnCharacteristicWrite; Apple waits `canSendWriteWithoutResponse` | — (library) | **Strongest candidate for a shared transport layer** (Kotlin Multiplatform; iOS via a KMP framework). Central role only. |
| [johnhowe/BlueTerm](https://github.com/johnhowe/BlueTerm) | Android, Java | **no LICENSE file** (jackpal-derived code inside) | 30 / 2014 | Classic SPP RFCOMM (UUID 0x1101) | stream | **Real VT100 emulator** (jackpal Term) | Historic proof of concept only; do not fork. |
| [treehouses/flutter-ble-terminal](https://github.com/treehouses/flutter-ble-terminal) | Flutter, Dart | no license | 4 / 2022 | flutter_blue 0.8, `write(withoutResponse:false)` | — | ListView of strings + TextField | Dead; a "Pi over BLE" command console UI, not a terminal. Sibling treehouses/remote (82★, AGPL-3.0) uses Bluetooth Classic. |
| [jonasneves/pi-loop](https://github.com/jonasneves/pi-loop) `bridge/bridge.py` | Server: Pi, Python (bless) | **no LICENSE** | 0 / 2026-10-22 | NUS server; PTY+bash; 20 B notify chunks | none | client = Web Bluetooth "tab-device" | Closest server analogue; no auth, no resize. |
| [monperrus/asteroid-ble-shell](https://github.com/monperrus/asteroid-ble-shell) | Server: AsteroidOS (Qt/C++); client: Python Bleak | GPL-3.0 | 0 / 2026-09-03 | NUS; `encrypt-authenticated-write/notify`; PTY `/bin/sh -i` as non-root | ? | Linux raw-tty client (Ctrl-C/D, ANSI pass-through), Ctrl-] escape | **Best security model** to copy; no resize. |
| [cocoahuke/coBlue](https://github.com/cocoahuke/coBlue) | Server: RPi BlueZ (C) | MIT | 43 / 2018 | custom service (Shell rw + File rw), MAC filter, UUID "password", `-secur` levels | ? | macOS client only; "iOS should also be able to support ... not written" | Historical; shows auth ideas (weak). |
| [hannesweisbach/ble-tty](https://github.com/hannesweisbach/ble-tty) | Linux, Python | MIT | 0 / 2021 | PTY to BLE peripheral | ? | — | Tiny reference. |
| [Jakeler/ble-serial](https://github.com/Jakeler/ble-serial) | Linux/Mac/Win PC, Python | MIT | 387 / 2024-12 | Central to virtual serial port (pty) | — | — | Useful **desktop test client** for our server (`screen /dev/pts/N`). |
| [migueldeicaza/SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) | iOS/macOS, Swift | MIT | 1740 / 2026-10-21 | transport-agnostic | — | Full xterm emulator, UIKit view | **iOS terminal UI candidate.** Its sample app uses SSH; supply our own BLE transport. |
| [termux/termux-app](https://github.com/termux/termux-app) `terminal-emulator` + `terminal-view` | Android, Java | app GPLv3; these two libs Apache-2.0 (LICENSE.md exception) | 63k / 2026-09 | transport-agnostic | — | Full xterm-ish emulator + view, extra keys | **Android terminal UI candidate.** Feed BLE bytes instead of a local PTY. Needs a custom `TerminalSession`-like class. [H] |

---

## 3. Source-level notes per project

### 3.1 SimpleBluetoothLeTerminal (Android, MIT) — `app/src/main/java/de/kai_morich/simple_bluetooth_le_terminal/`

- **Service autodetect [V]** `SerialSocket.java:48-64`. UUIDs for CC254x/HM-10 (FFE0/FFE1), NUS (6E400001/2/3), Microchip, Telit TIO.
  - `NrfDelegate` (`:476-504`) picks the write characteristic *by property*, not by UUID. Comment: "read on microbit, write on adafruit". So the RX/TX naming of NUS is ambiguous in practice, and a robust client should bind by property.
- **Connect sequence [V]** `:152-166` `connectGatt(..., TRANSPORT_LE)`, then discoverServices (`:201`), then `requestMtu(MAX_MTU=512)` (`:66, :258`).
  - `onMtuChanged` sets `payloadSize = mtu-3` (`:267-275`).
  - Then it enables notify or indicate by writing the CCCD (`:278-310`).
  - It marks "connected" only after `onDescriptorWrite` (`:313-329`). The comment there notes that notifications can arrive *before* the CCCD write is confirmed.
- **Pairing [V]** `:172-194`. A bonding broadcast receiver. A pairing request aborts the connect with an error, and the user is told to pair from system settings. Crude.
- **Write path [V]** `write()` `:349-384`, `onCharacteristicWrite` `:387-399`, `writeNext` `:403-421`.
  - Chunks to payloadSize under a `synchronized(writeBuffer)` ArrayList queue. Only one GATT write is in flight at a time.
  - The write type is the characteristic default. For NUS RX, which has both WRITE and WRITE_NO_RESPONSE, Android's default is with response. [H, based on Android framework behaviour]
- **Telit TIO credits [V]** `:506-645`. Bidirectional credit flow control: `grantReadCredits` keeps 16..64 credits, and `canWrite()` checks `writeCredits>0`. This is a clean, already-implemented model of application-level backpressure over GATT.
- **Read path [V]** `onCharacteristicChanged` `:333-345` uses the deprecated `getValue()` (no API-33 `value` overload).
- **Background [V]** `SerialService.java:182` `startForeground`. Data is queued while the UI is detached (`:24, :110-132`).
- **UI [V]** `TerminalFragment.java:131-133` TextView log. `:248-275` decodes each packet with `new String(data)`, does CR/LF folding, and `TextUtil.toCaretString` (`TextUtil.java:79-`) shows control chars as `^X`. **No ANSI parsing.** Sending is line plus a configurable newline (`:229-237`).
- **Reconnection:** none. On error it disconnects and the user reconnects manually.

### 3.2 nRF Toolbox Android (BSD-3) — UART profile

- `profile_manager/.../manager/UARTManager.kt:20-21` RX=6E400002 (write), TX=6E400003 (notify). `:35-40` validates the properties.
- Receive [V] `:42-51`: `txCharacteristic.subscribe().map { String(it) }`. Per-packet UTF-8 decode.
- Send [V] `:56-69`: `message.toByteArray().chunked(maxWriteLength).forEach { rxCharacteristic.write(it) }`.
  - `maxWriteLength` comes from `peripheral.maximumWriteValueLength(WriteType.WITHOUT_RESPONSE)` (`profile/.../ProfileScreen.kt:227,231`).
  - The write type is the library default, which is `WITH_RESPONSE` when the WRITE property is present (Kotlin-BLE-Library `core/.../CharacteristicProperty.kt:118-119`; `client-core/.../RemoteCharacteristic.kt:172-175`).
- MTU [V]: `lib_service/.../ProfileService.kt:144-146` `ConnectionOptions.Direct(automaticallyRequestHighestValueLength = true)`.
- Reconnect: a manual "Reconnect" button (`ProfileScreen.kt:173`).
- UI: Compose `LazyColumn` of messages (`view/uart/OutputSection.kt:67`), plus macros. Not a terminal.

### 3.3 nRF Toolbox iOS (BSD-3)

- `nRF-Toolbox/Screens/Services/UART/UARTViewModel.swift`:
  - `:118-119` `setNotifyValue(true)`.
  - `:142-164` `send()` calls `writeValueWithResponse(data)` for the **entire message without chunking**. [H] CoreBluetooth then does a long (prepared) write, up to 512 bytes.
  - `:167-185` receive: `String(data:encoding:.utf8)`, which returns `nil` and drops the packet on invalid UTF-8. The text is then `trimmingCharacters(in: whitespacesAndNewlines)`.
- `Throughput/ThroughputViewModel.swift:67` `maximumWriteValueLength()`. `:176` `await peripheral.isReadyToSendWriteWithoutResponse()` before each `writeValueWithoutResponse`. This is the correct iOS pattern, shown in nordicsemi/IOS-BLE-Library (BSD-3, 108★).
- Dependencies (pbxproj): IOS-BLE-Library, IOS-CoreBluetooth-Mock, IOS-Common-Libraries.

### 3.4 Bluefruit LE Connect iOS v2 (MIT) — `Bluefruit/AdafruitKit/Ble/BleCentralMode/`

- `BlePeripheral+Uart.swift:88`: the write type is `.withoutResponse` if the characteristic supports it.
- `:151-189` chunks at `maximumWriteValueLength(for: .withoutResponse)` (comment: use this "even if sending .withResponse").
- `BlePeripheral.swift:506-513`: the `.withoutResponse` branch loops `peripheral.writeValue(chunk, type: .withoutResponse)` with **no `canSendWriteWithoutResponse` check**. It then immediately calls `finishedExecutingCommand`. [H] Bursts beyond the CoreBluetooth queue are silently dropped.
- `BlePeripheral+Uart.swift:191-235` `uartSendEachPacketSequentially(withResponseEveryPacketCount:)` makes every Nth packet `.withResponse` "to force wait ... avoid dropping packets". A pragmatic hybrid.
- `BlePeripheral.swift:173-` a serial `CommandQueue` for GATT operations. It also has a GATT *server* (peripheral) mode, `BlePeripheralMode/GattServer.swift`.

### 3.5 Bluefruit LE Connect Android V2 (MIT, archived)

- `ble/central/BlePeripheral.java:48-50,117-121` requests MTU 517 and 2M PHY on connect.
- `:760-790` a command queue. For WRITE_TYPE_NO_RESPONSE it waits for `onCharacteristicWrite`, citing a StackOverflow answer that Android calls it for no-response writes once the packet is buffered (`:181-185` comments). [V comment; the Android behaviour itself is H/known]
- `BlePeripheralUart.java:45` `UART_PACKET_SEND_DELAY_MS = 40`, `:234-256` with-response every N packets plus a delay.

### 3.6 Kable (Apache-2.0, KMP)

- Apple `kable-core/src/appleMain/kotlin/CBPeripheralCoreBluetoothPeripheral.kt:213-229`. WithResponse awaits `DidWriteValueForCharacteristic`. WithoutResponse takes a `guard` mutex, checks `cbPeripheral.canSendWriteWithoutResponse`, and otherwise suspends until `PeripheralDelegate.peripheralIsReadyToSendWriteWithoutResponse` (`PeripheralDelegate.kt:241-247`) flips the StateFlow. There is a configurable timeout (`PeripheralBuilder.kt:64`).
- Android `kable-core/src/androidMain/kotlin/BluetoothDeviceAndroidPeripheral.kt:195-211`. Every write (any type) is `connection.execute<OnCharacteristicWrite>`, serialized by a `Mutex` (`Connection.kt:102,150`). So `onCharacteristicWrite` gives natural backpressure. `requestMtu` is at `:187`. `maximumWriteValueLengthForType` is at `:171`. The doc at `commonMain/Peripheral.kt:170` says Android stays at 20 bytes unless `requestMtu` is called.
- Takeaway: one Kotlin codebase could own the BLE transport, framing and reconnection logic for both apps. The UI (Termux view / SwiftTerm) stays native. [H: KMP-to-Swift interop cost and SwiftTerm integration not evaluated]

### 3.7 Server-side "shell over BLE" precedents (clients are not mobile)

- **pi-loop** `bridge/bridge.py` (no license):
  - `:32-35` `pty.fork()` + execvp.
  - `:39-40` RX write goes to `os.write(master_fd)`.
  - `:44-51` NUS characteristics (RX write+WwR, TX notify).
  - `:56-65` reads the PTY 256 B at a time and notifies in **20 B chunks** with no flow control.
  - There is no pairing or encryption requirement: README says "live root shell over Bluetooth with zero config".
  - No window size (TIOCSWINSZ) handling.
  - Client: Web Bluetooth page ("tab-device").
- **asteroid-ble-shell** (GPL-3.0), README:
  - RX `encrypt-authenticated-write`, TX `encrypt-authenticated-notify`.
  - The peer must be paired *and* trusted.
  - Local "arm" for 15 min.
  - Input is accepted only after the TX subscription.
  - The peer identity is pinned per session.
  - The shell runs as non-root.
  - Disconnect kills the PTY process group.
  - "PTY starts at 80×24. Baseline NUS has no terminal-resize control channel."
  - Client: `client/ble-uart.py` (Bleak) raw mode, Ctrl-] to quit.
  - Also notes a BlueZ D-Bus disconnect race and stuck `bluetoothd` recovery.
- **coBlue** (MIT, 2018): BlueZ-based C daemon with "Shell Terminal (rw)" and "File Transfer (rw)" characteristics, MAC filtering, a UUID string sent within 3 s as a "password", and `-secur low|medium|high|fips`. The author wrote: "iOS should also be able to support ... but i have not written a program for iOS yet".
- **hannesweisbach/ble-tty** (MIT, 2021): "Service to create a PTY and transport I/O to a BLE peripheral".
- **Zephyr / nRF Connect SDK `CONFIG_SHELL_BT_NUS`**: an MCU shell over NUS with TX/RX ring buffers ([Kconfig](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/v1.6-branch/kconfig/CONFIG_SHELL_BT_NUS.html), [API](https://nrfconnectdocs.nordicsemi.com/ncs/latest/nrf/doxygen/html/shell__bt__nus_8h.html)). Usually consumed from desktop scripts or nRF Toolbox UART. This is the most common "shell over BLE" in the wild, but it is an MCU shell, not Linux.

### 3.8 Bluetooth Classic precedents (the "old way")

- `rfcomm watch hci0 1 getty rfcomm0 115200 vt100` as a systemd unit ([blog.habets.se 2022](https://blog.habets.se/2022/01/Raspberry-Pi-Bluetooth-console.html)).
  - Caveats: no PIN, so login brute-force is possible.
  - `rfcomm watch` busy-waits at about 35% CPU.
  - The service races bluetoothd at boot.
  - The client is a laptop running minicom/screen. Also on [RPi forum t=144896](https://forums.raspberrypi.com/viewtopic.php?t=144896) and [historiantech](https://www.historiantech.com/create-a-bluetooth-wireless-console-server-with-raspberry-pi-zero-w/).
- Android SPP terminal apps (BlueTerm with VT100, Serial Bluetooth Terminal line-mode) consumed these. **iOS cannot use SPP/RFCOMM without MFi.** [H, well known] This is why BLE is required for an iOS client.
- treehouses/remote (Android, AGPL-3.0, 82★) controls a Pi over Bluetooth Classic with a command console. treehouses/control is the Python RFCOMM server.

### 3.9 External UART bridge (HM-10 / ESP32 to Pi getty)

- No documented end-to-end project was found where a phone drives a Linux getty through an HM-10 or ESP32 BLE-UART bridge with full-screen apps. Searches returned only generic HM-10 serial apps (e.g. "BLESerial HM-10" iOS) and Espressif `AT+BLESPP` docs. [V: search result absence; H: likely exists informally in forums]
- Bridge firmware exists: olegv142/esp32-ble-uart-mx (Unlicense), vedderb/nrf52_vesc, afpineda/NuS-NimBLE-Serial, senseshift/arduino-ble-serial.
- [H] Assessment: a bridge adds a second flow-control boundary (UART baud plus BLE). It usually lacks RTS/CTS over BLE and cannot carry window size. It is only attractive when the Linux host has no usable BLE radio.

---

## 4. Implications for the study (hypotheses for the architecture doc)

- [H] **Client composition.**
  - Android: Termux `terminal-emulator`/`terminal-view` (Apache-2.0) + Kotlin-BLE-Library or Kable.
  - iOS: SwiftTerm (MIT) + CoreBluetooth directly or Kable-via-KMP. Use IOS-CoreBluetooth-Mock for tests.
  - Desktop test client: ble-serial, or a Bleak client like asteroid's `ble-uart.py`.
- [H] **Do not use bare NUS as the wire protocol.**
  - Every surveyed project shows the gaps: no resize, no session, no backpressure from server to client, and no binary-safe framing for control messages.
  - Options: either a second "control" characteristic (resize, signals, credits; Telit TIO's credits characteristic is precedent), or in-band framing on the data characteristic.
  - NUS-compatible fallback mode still makes sense so stock apps (nRF Toolbox, Serial Bluetooth Terminal) can at least get a line shell.
- [H] **Flow control rules for the client:**
  - Android: one outstanding write, advanced by `onCharacteristicWrite`, for both write types.
  - iOS: gate `.withoutResponse` on `canSendWriteWithoutResponse` / `peripheralIsReady(toSendWriteWithoutResponse:)`.
  - Chunk to `maximumWriteValueLength(.withoutResponse)` (iOS) or `mtu-3` (Android). Request MTU 517 on Android; iOS negotiates automatically.
- [H] **Decoding:** never decode text per notification. Pass raw bytes to the emulator, which owns the UTF-8 state.
- [H] **Security:** copy asteroid-ble-shell's model: authenticated encrypted characteristics (LESC pairing), a bonded-and-trusted peer only, input only after subscription, a non-root or `login` session, a PTY kill on disconnect, and an optional local "arm" window.

## 5. Sources

- GitHub repos and commits as listed in the table at top; metadata via `gh api repos/OWNER/REPO` on 2026-10-24.
- [Serial Bluetooth Terminal – Google Play](https://play.google.com/store/apps/details?id=de.kai_morich.serial_bluetooth_terminal&hl=en_US); [APKMirror listing](https://www.apkmirror.com/apk/kai-morich/serial-bluetooth-terminal/) (line-oriented terminal; v1.48 nRF WwR).
- [Bluetooth Terminal iOS (App Store)](https://apps.apple.com/us/app/bluetooth-terminal/id1058693037); [BLESerial HM-10 (App Store)](https://apps.apple.com/us/app/-/id1602239700). Both are prototyping/serial monitors, not VT emulators.
- [blog.habets.se — Raspberry Pi bluetooth console (2022)](https://blog.habets.se/2022/01/Raspberry-Pi-Bluetooth-console.html).
- [Nordic CONFIG_SHELL_BT_NUS](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/v1.6-branch/kconfig/CONFIG_SHELL_BT_NUS.html).
- [Ineltek: Bluetooth LE SPP (ESP AT+BLESPP)](https://docs.ineltek.com/docs/4-bluetooth-le-spp/).
- termux-app LICENSE.md (Apache-2.0 exception for terminal-emulator/terminal-view), fetched via gh api.
