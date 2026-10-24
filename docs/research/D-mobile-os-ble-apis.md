> Research working notes (2026-10-24), appendix to `../ble-terminal-research.md`. Paths like `scratchpad/repos/...` refer to temporary clones; line numbers match the SHAs given.

# D. Mobile OS BLE API constraints (iOS CoreBluetooth / Android) for a BLE terminal

Scope: BLE terminal with a Linux/BlueZ peripheral (Raspberry Pi) and phone apps as central. Research date: 2026-10-24.
Labels: **[V]** = verified from a primary source (Apple doc/ADG/WWDC transcript, Android doc, AOSP source). **[S]** = secondary source (vendor blog, forum, measurement). **[H]** = hypothesis or inference to validate on hardware.

## Primary sources used

- Apple **Accessory Design Guidelines for Apple Devices, Release R31** (PDF dated 2026-09-21), ch. 58 "Bluetooth Low Energy (BLE)", §58.6–58.11 (pp. 352–354). https://developer.apple.com/accessories/Accessory-Design-Guidelines.pdf
- **WWDC17 Session 712 "What's New in Core Bluetooth"**. Apple has removed the video page, so the transcript comes from https://asciiwwdc.com/2017/sessions/712
- Apple CoreBluetooth reference, read through the doc JSON at `developer.apple.com/tutorials/data/documentation/corebluetooth/...`: `openL2CAPChannel(_:)`, `CBUUIDL2CAPPSMCharacteristicString`, `canSendWriteWithoutResponse`, `peripheralIsReady(toSendWriteWithoutResponse:)`, `writeValue(_:for:type:)`, `connect(_:options:)`, `CBConnectPeripheralOptionEnableAutoReconnect`, `CBCentralManagerOptionRestoreIdentifierKey`, `centralManager(_:willRestoreState:)`, `publishL2CAPChannel(withEncryption:)`, `CBPeer.identifier`, `CBATTError.insufficientAuthentication`.
- Apple Core Bluetooth Programming Guide, "Core Bluetooth Background Processing for iOS Apps": https://developer.apple.com/library/archive/documentation/NetworkingInternetWeb/Conceptual/CoreBluetooth_concepts/CoreBluetoothBackgroundProcessingForIOSApps/PerformingTasksWhileYourAppIsInTheBackground.html
- Apple Technical Q&A **QA1962**, "Conditions Under Which Bluetooth State Restoration Will Relaunch An App": https://developer.apple.com/library/archive/qa/qa1962/_index.html
- Android 14 behavior changes (all apps), "MTU is set to 517": https://developer.android.com/about/versions/14/behavior-changes-all
- Android 14 FGS types: https://developer.android.com/about/versions/14/changes/fgs-types-required
- Android Bluetooth permissions: https://developer.android.com/develop/connectivity/bluetooth/bt-permissions
- Android companion device pairing: https://developer.android.com/develop/connectivity/bluetooth/companion-device-pairing
- **AOSP** `platform/packages/modules/Bluetooth` (main branch, fetched 2026-10-24 from android.googlesource.com):
  - `framework/java/android/bluetooth/BluetoothGatt.java`
  - `framework/java/android/bluetooth/BluetoothGattCallback.java`
  - `framework/java/android/bluetooth/BluetoothDevice.java`
  - `framework/java/android/bluetooth/BluetoothSocket.java`
  - `framework/java/android/bluetooth/BluetoothStatusCodes.java`
  - `android/app/src/com/android/bluetooth/gatt/GattService.java`
  - `android/app/res/values/config.xml`
- App Store Review Guidelines: https://developer.apple.com/app-store/review/guidelines/
- MFi FAQ: https://mfi.apple.com/faqs and https://mfi.apple.com/en/who-should-join

---

## 1. MTU, DLE, PHY

### iOS
- **[V] No app API exists to request an ATT MTU.** ADG §58.11: "When operating as ATT client, the device will request the optimal MTU size based on factors such as the Bluetooth topology, connection event length, maximum data length, and protocol (GATT or connection-oriented L2CAP). An accessory operating as ATT server should select an MTU equal to or greater than the device's MTU request." WWDC17-712 adds: "We will calculate the MTU for you…"
- **[V] The ADG asks the accessory to do the DLE procedure before the MTU exchange.** §58.11: "An accessory supporting packet length extension shall perform the packet length update procedure before performing the Exchange MTU Request handshake." §58.7: DLE goes from 27 to 251 bytes, and "iOS devices and Mac computers operating as the Central will negotiate optimal data packet lengths". WWDC17: DLE is available from iPhone 7 / Apple Watch S2.
- **[V] The app learns the usable payload size from `maximumWriteValueLength(for:)` (iOS 9+).** For `.withResponse` it returns 512 because long writes are allowed. For `.withoutResponse` it returns ATT_MTU−3, which is the value to use for chunk size. **[S]** Forum thread 81120 (iOS 11, iPhone 7) reports `maximumWriteValueLength` = 512.
- **[S] Observed iOS ATT MTU values vary by model and iOS version, and Apple publishes no constant:**
  - 185 on iOS 10-era devices (Nordic DevZone 44825).
  - 247 on iPhone 7 (Apple forum 81120).
  - 527 "or something higher, based on the iPhone" up to iOS 15.
  - From iOS 16.1.1, one report says the iPhone sent no Exchange MTU request at all, so the peripheral's value (250) applied (Apple forum 721541).
  - **Design rule:** never hard-code an MTU; read `maximumWriteValueLength(.withoutResponse)` after connecting, and have BlueZ offer a large MTU (≥ 517).
- **[V] CBL2CAPChannel MTU/MPS/SDU cannot be set from the iOS central.** No API exists for it. **[S]** Forum thread 723218 (FB13742028) asks for one. Forum 81120 observed an L2CAP MTU of 512 on iPhone 7, with data fragmented above 2 KB.
- **[S] 2M PHY:** iOS exposes no PHY API. The Apple DTS forum reply lists "PHY2" as something the accessory requests ("connection interval, MTU, PHY can only be requested by the accessory"; forum 770717/773555). The ADG chapter for spatial accessories requires "LE 2M PHY rate", which implies current iPhones support it. **[H]** 2M is used on iPhone 8/X and later when the peripheral initiates the LE PHY Update.

### Android
- **[V] `BluetoothGatt.requestMtu(int)` exists; from Android 14 the stack always asks for 517.** Android 14 notes: "requests the BLE ATT MTU to 517 bytes when the first GATT client requests an MTU using the BluetoothGatt#requestMtu(int) API, and disregards all subsequent MTU requests on that ACL connection." The final value is `min(517, remoteMtu)`, delivered in `onMtuChanged`. The AOSP Javadoc (`BluetoothGatt.java`) repeats this.
- **[V] One attribute value is capped at 512 bytes.** `writeCharacteristic` throws `IllegalArgumentException` if `value.length > GATT_MAX_ATTR_LEN` (`BluetoothGatt.java`). The 517 MTU minus 5 header bytes is the recommended per-write maximum (Android 14 doc).
- **[V] Before Android 14, MTU stays 23 until an app calls `requestMtu`.** The app has to request it.
- **[V] `setPreferredPhy(txPhy, rxPhy, phyOptions)` and `readPhy()` exist on BluetoothGatt.** The `connectGatt(..., phy)` overloads arrived in API 26. 2M support depends on the controller: `BluetoothAdapter.isLe2MPhySupported()`.
- **[V] DLE is not exposed to apps.** The stack and controller negotiate it. **[S]** DLE support varies by handset. Memfault and Punch Through note that "some devices perform quite well, others very poorly".

## 2. Connection interval and throughput

### Connection interval
- **[V] iOS limits (ADG R31 §58.6), given as connection-parameter request guidelines.** iOS "will not read or use the parameters in the Peripheral Preferred Connection Parameters characteristic", so the peripheral must send an LL/L2CAP Connection Parameter Update request. Requests "may be rejected if they do not meet these guidelines":
  - Interval Min ≥ 15 ms, and a multiple of 15 ms.
  - Interval Max ≥ Min + 15 ms, or Min = Max = 15 ms.
  - Peripheral Latency ≤ 30.
  - Supervision timeout from 6 s to 18 s, and > Interval Max × (Latency + 1) × 3.
  - Interval Max × (Latency + 1) ≤ 6 s.
  - "If Bluetooth Low Energy HID is one of the connected services of an accessory, a connection interval down to 11.25 ms may be accepted by some devices."
  - With Min = Max = 15 ms, "some devices (such as Apple Watch) will offer a 30 ms interval".
- **[S] iOS may still pick other parameters.** Apple DTS (forum 770717): "the connection parameters the peripheral requests are not guaranteed to be accepted as-is… you are stuck with whatever iOS will accept. You would have no opportunity to influence this via your app."
- **[H] Exposing a HID service from the Pi only to get 11.25 ms is not worthwhile.** iOS would then treat the Pi as a keyboard, with side effects such as the on-screen keyboard being hidden.
- **[V] Android `requestConnectionPriority()` values, from AOSP `android/app/res/values/config.xml`** (units of 1.25 ms):

  | Priority | Interval | Latency |
  |---|---|---|
  | HIGH | 9–12 → **11.25–15 ms** | 0 |
  | BALANCED (default) | 24–40 → **30–50 ms** | 0 |
  | LOW_POWER | 80–100 → 100–125 ms | 2 |
  | DCK (API 34+) | 24 → 30 ms | 0 |

  - Companion-device "primary" profiles get 7.5–10 ms intervals with high latency values.
  - The AOSP doc says to use HIGH only "to transfer large amounts of data… Once the transfer is complete… request BALANCED."
  - `requestLeConnectionUpdate(min,max,...)` is `@hide`, so apps cannot set an exact interval.
  - **[S]** OEMs can override `config.xml`.
- **[H] For a terminal session:** request HIGH while the session is active (11.25–15 ms gives low keystroke-echo latency) and drop to BALANCED when idle. On iOS, have the Pi request 15 ms (min 15 / max 15, or 15/30).

### Throughput numbers
| Source | Setup | Number |
|---|---|---|
| **[V] WWDC17-712 (Apple)** | iOS, 30 ms CI: WwR 2.5 kbps → WNR packed 37 → large MTU 48 → +DLE 135 → **L2CAP+DLE ≈200 kbps** → L2CAP+DLE+15 ms CI **≈394 kbps (~49 KB/s)** | "20 seconds to download 1 MB" |
| [S] Apple DTS forum 770717 | DLE lets "an entire maximum GATTs write of 512 Bytes in a single interval", "about 3 times" | — |
| [S] Punch Through FAQ (2026-05-11) | Mobile device on one end | **90–100 kB/s** typical max. Without DLE ~54 kB/s. They observed "iOS allocating roughly 50% of a 15 ms connection interval to BLE". Android "shows more variability". Best ATT payloads 244 or 495 bytes |
| [S] Punch Through Part 4 (2020) | Newer Android/iOS, DLE, 1M PHY | ~50 KB/s |
| [S] Punch Through (older) | Packets per connection event | ~4 iOS, ~6 Android |
| [S] Apple forum 723218/81120 (2024) | iPhone central, L2CAP CoC | ~36 KB/s default, ~60 KB/s with 15 ms CI. iPhone as peripheral ~100 KB/s. Another user saw 18 KB/s with BTstack CoC |
| [S] Nordic DevZone 61028 | nRF↔iOS, GATT | ~65 KB/s. Nordic engineer: L2CAP gives "no significant" gain over GATT (3-byte ATT header only) |

- **Conclusion for a terminal [H]:** about 30–100 KB/s is achievable on either transport. A full 200×60 xterm repaint is about 12–40 KB with SGR sequences, so it takes about 0.2–1 s at worst. vim/htop incremental updates are much smaller.
- **Latency matters more than throughput.** It is bounded by the connection interval: about 15–30 ms one-way, plus the OS scheduling slot, giving an RTT for a keystroke echo of about 30–60 ms. Throughput is not the bottleneck. Flow control (backpressure from a slow phone during `cat bigfile`) is the bottleneck.

## 3. Flow control / backpressure

### iOS
- **[V] Write Without Response, central→peripheral:**
  - `canSendWriteWithoutResponse` (iOS 11+): "If this value is false, flushing all current writes sets the value to true. This also results in a call to the delegate's peripheralIsReady(toSendWriteWithoutResponse:)".
  - WWDC17: when it returns YES, "that's our promise to you that your data will not be dropped in software before we get a chance to send it". Before iOS 11, WNR writes could be dropped silently.
  - `writeValue(_:for:type:)` doc: with `.withoutResponse` "Core Bluetooth attempts to write the value but doesn't guarantee success… you aren't notified".
  - **The keystroke path should therefore gate on `canSendWriteWithoutResponse`.**
- **[V/H] Notifications, peripheral→central:**
  - Core Bluetooth has no API to throttle, pause, or acknowledge notifications. `peripheral(_:didUpdateValueFor:error:)` simply fires for each one.
  - The only backpressure is in the link layer (controller buffers) and is invisible to the app.
  - **[H]** If the app's delegate queue is slow, iOS keeps queuing in memory. The app must buffer, and a terminal emulator must parse fast.
  - **Application-level credit flow control is needed.** Example: the central periodically writes a "window/ack" (bytes consumed). The Pi stops reading the PTY when the window is exhausted, so the kernel PTY buffer then blocks the producer, giving natural `^S`-like backpressure.
  - **Indications** are acknowledged one per round trip, which costs about 1 PDU per 2 connection intervals. That is too slow for bulk output, but acceptable for a control channel.
- **[V] L2CAP CoC (CBL2CAPChannel):**
  - Provides `inputStream`/`outputStream` (Foundation `InputStream`/`OutputStream`). WWDC17: "we just give you an InputStream and OutputStream… If you already know how to deal with the socket…".
  - The app writes on `.hasSpaceAvailable` and reads on `.hasBytesAvailable` stream events, scheduled on a run loop.
  - **[H] Backpressure comes from LE credit-based flow control.** If the app does not read the input stream, iOS stops returning credits and the Pi's `write()` on the L2CAP socket eventually blocks or returns EAGAIN. The amount iOS buffers internally before withholding credits is undocumented, so measure it.

### Android
- **[V] Only one outstanding GATT operation per BluetoothGatt.** `BluetoothGatt.java` keeps an `mDeviceBusy` flag:
  - `writeCharacteristic(char, value, writeType)` (API 33) returns `BluetoothStatusCodes.ERROR_GATT_WRITE_REQUEST_BUSY` (201) if a previous op is pending.
  - The read/descriptor ops return `false`.
  - It internally retries the service call up to 5×10 ms (`WRITE_CHARACTERISTIC_MAX_RETRIES=5`, `WRITE_CHARACTERISTIC_TIME_TO_WAIT=10`).
  - **[V]** `GattService.java` adds a **per-remote-address permit** (`mPermits`) shared across all apps. A second write before `onWriteCharacteristic` returns `ERROR_GATT_WRITE_REQUEST_BUSY`.
- **[V] WNR flow control is the `onCharacteristicWrite` callback, which arrives even for WRITE_TYPE_NO_RESPONSE.**
  - In `GattService.onWriteCharacteristic`, when the link is congested (`app.isCongested`, set by `onClientCongestion`), the callback is **queued and delivered only when congestion clears**.
  - So "wait for onCharacteristicWrite before the next write" is real backpressure.
  - Consequence: at most one WNR in flight from the app at a time. Throughput is still good because the callback fires when the stack has buffered the packet, not when it is acknowledged over the air.
- **[V] Notifications:**
  - The API 33+ callback is `onCharacteristicChanged(gatt, characteristic, byte[] value)`. The old variant is deprecated because the characteristic object's value can change under you.
  - `runOrQueueCallback` runs the callback directly on the Binder thread when no Handler was passed to `connectGatt`, otherwise it posts to the Handler.
  - No app-level throttling exists. Copy the bytes and hand them off to a parser thread quickly.
  - **[H]** Never block the binder callback thread.
- **[V] L2CAP CoC (`BluetoothSocket` TYPE_L2CAP_LE):**
  - Uses blocking `InputStream`/`OutputStream`.
  - `BluetoothSocket.write` splits data into `mMaxTxPacketSize` chunks (`getMaxTransmitPacketSize()`/`getMaxReceivePacketSize()`).
  - **[H]** Credit-based flow control gives real backpressure on both directions: a blocking `write()` stalls when the Pi has not granted credits, and not reading `InputStream` stops credits going back to the Pi.

## 4. L2CAP CoC availability and interop

- **[V] iOS 11+ `CBPeripheral.openL2CAPChannel(_ PSM:)`.**
  - The result arrives in `peripheral(_:didOpen:error:)`.
  - The PSM must be learned out of band. Apple defines `CBUUIDL2CAPPSMCharacteristicString` = `ABDD3056-28FA-441D-A470-55A75A52553A`: "The PSM of an L2CAP channel associated with the GATT service containing this characteristic… a little-endian UInt16." **Use this UUID in the Pi's GATT service.**
  - `publishL2CAPChannel(withEncryption:)` (peripheral side): "L2CAP channels aren't discoverable by themselves, so it's the app's responsibility to handle PSM discovery."
- **[V/H] iOS central encryption requirement.**
  - `openL2CAPChannel` has no encryption parameter. Encryption is decided by the server (the Pi): BlueZ socket `BT_SECURITY` level (LOW = no encryption).
  - WWDC17 states the peripheral chooses whether a channel requires encryption.
  - **[H]** An unencrypted CoC from iOS central to a BlueZ listener with `BT_SECURITY_LOW` should open without pairing. If the Pi requires MEDIUM/HIGH, the LE CoC connection response returns "insufficient authentication/encryption" and iOS should start pairing. That behavior is not documented by Apple, so test it.
  - **[S]** Forum 675960: iOS central → Android secure CoC failed with "Unknown ATT error 104", while the insecure channel worked.
- **[V] Android API 29+:**
  - `BluetoothDevice.createL2capChannel(psm)` doc: "The remote device will be authenticated and communication on this socket will be encrypted." That means bonding is needed.
  - `createInsecureL2capChannel(psm)`: "may not have an authenticated link key, i.e. it may be subject to person-in-the-middle attacks".
  - Both: "The supported Bluetooth transport is LE only… Application using this API is responsible for obtaining PSM value from remote device."
  - Implementation: `new BluetoothSocket(this, TYPE_L2CAP_LE, auth, encrypt, psm, null)` in `BluetoothDevice.java`.
- **[V] BlueZ side.** LE CoC uses an `AF_BLUETOOTH`/`BTPROTO_L2CAP` `SOCK_SEQPACKET` socket bound to an LE address type with a dynamic PSM (0x0080–0x00FF). The mode is `BT_MODE_LE_FLOWCTL` (0x03, Linux ≥ 5.10 via the `BT_MODE` sockopt; earlier kernels select LE CoC implicitly from the LE address type). Sources: bluez/bluez `doc/l2cap-protocol.rst` and the l2cap(7) man page.
- **[S] Interop evidence, iOS/Android ↔ BlueZ CoC.**
  - No published, verified end-to-end iOS→BlueZ CoC success report was found.
  - Adjacent evidence:
    - Pears `bare-bluetooth` offers L2CAP channels on Linux (BlueZ), macOS, iOS, and Android in one API. Unencrypted is the default, plus a GATT characteristic for PSM discovery, and they warn that "opening a channel can hang without an error – set a timeout".
    - The `blew` Rust crate uses `bluer` on Linux with iOS `publishL2CAPChannel(withEncryption:false)` and Android insecure CoC.
    - Forum 723218 reports a BlueZ `l2test` (Basic mode, wrong mode) failure against iPhone.
  - **Treat CoC interop as plausible but unproven. Prototype it early.**
- **[S] Known iOS CoC bugs.**
  - Multiple channels on the same peripheral get mixed up (rdar://46227689, forum 111099). The workaround is one CBCentralManager per channel. Reported again in 2025.
  - Channel parameters cannot be tuned (forum 723218).
  - **[H]** Use a single CoC per connection.
- **[S] Android CoC fragmentation.**
  - CoC has been supported since Android 10 (Nordic DevZone 100822).
  - Reports of secure listen variants never yielding a socket (forum 675960).
  - Reports of Pixel 6/8 (Android 14/15) failing against AirPods-type peers because of PSM range and channel type (capod #215).
  - No systematic Samsung bug list was found.
  - **[H]** Keep GATT as the mandatory transport and make CoC an optional upgrade negotiated over GATT.
- **[S] BlueZ/iOS GATT interop gotchas** (Ditto docs):
  - BlueZ ≥ 5.51 battery plugin reads trigger repeated iOS pairing prompts. Fix: run `bluetoothd -P battery`.
  - Set `ControllerMode = le` in `main.conf` for iOS/macOS interop.

## 5. Background operation and reconnection

### iOS
- **[V] `bluetooth-central` UIBackgroundMode** (Programming Guide):
  - "the system wakes up your app when any of the CBCentralManagerDelegate or CBPeripheralDelegate delegate methods are invoked… when a peripheral sends updated characteristic values".
  - Background scans coalesce duplicates and scan less often.
  - "Upon being woken up, an app has around 10 seconds to complete a task… Apps that spend too much time executing in the background can be throttled back by the system or killed."
- **[V] State preservation/restoration** (`CBCentralManagerOptionRestoreIdentifierKey`, iOS 7+; `centralManager(_:willRestoreState:)`):
  - The system relaunches the app "to service active or pending connections and scans that were in progress".
  - Preserved state covers scanned services, pending/connected peripherals, and subscribed characteristics.
  - In scene-based apps `launchOptions` is nil, so persist the UID yourself.
- **[V] QA1962, relaunch matrix (iOS 11+):**
  - YES: removed from memory; crashed; Airplane-mode toggle (if BT is not toggled); device restart (after first unlock).
  - **NO: user force-quit; Bluetooth power toggled.**
  - **[S]** Punch Through claims a device restart also prevents relaunch, which contradicts QA1962. Trust QA1962.
- **[V] `connect(_:options:)`: "Attempts to connect to a peripheral don't time out."** A pending connect is the standard way to auto-reconnect when the Pi comes back in range, and it survives restoration.
- **[V] `CBConnectPeripheralOptionEnableAutoReconnect` (iOS 17+):** "enables the system to initiate a connection to the peer device automatically when the link drops", with `didDisconnectPeripheral…isReconnecting:`.
- **[H] For a terminal:**
  - A backgrounded app with `bluetooth-central` keeps the GATT link and receives notifications. Each notification wakes the app, so output is parsed while backgrounded.
  - It is unclear and undocumented whether **CBL2CAPChannel stream data wakes a suspended app**. The streams are run-loop based and no doc says so. **Use GATT notifications as the background path, or at least a GATT "doorbell" notification.**
  - An interactive terminal is mostly foreground. The design goal is session persistence across backgrounding: keep the shell/tmux-like session on the Pi, then reconnect and replay the screen.
- **[S] watchOS/tvOS** (WWDC17): connections are dropped when the app is suspended. iOS behaves differently.

### Android
- **[V] FGS type `connectedDevice` for apps targeting 14+.**
  - Manifest permission `FOREGROUND_SERVICE_CONNECTED_DEVICE`.
  - Runtime prerequisite: one of `BLUETOOTH_CONNECT`/`BLUETOOTH_ADVERTISE`/`BLUETOOTH_SCAN` granted (or CHANGE_NETWORK_STATE, etc.).
  - Description: "Interactions with external devices that require a Bluetooth, NFC, IR, USB, or network connection."
  - **A foreground service plus a persistent notification is the robust way to hold a session in the background.**
- **[V] Android 12+ permissions:**
  - `BLUETOOTH_SCAN` (with `usesPermissionFlags="neverForLocation"` to avoid needing location) and `BLUETOOTH_CONNECT`, both runtime permissions.
  - Legacy `BLUETOOTH`/`BLUETOOTH_ADMIN` with `maxSdkVersion=30`.
  - Android ≤ 11 needs ACCESS_FINE_LOCATION for scanning.
- **[V] CompanionDeviceManager:**
  - Scans without ACCESS_FINE_LOCATION (API 26+) and shows a system chooser dialog.
  - After association, `REQUEST_COMPANION_RUN_IN_BACKGROUND`, `REQUEST_COMPANION_USE_DATA_IN_BACKGROUND`, and `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND` become available.
  - `startObservingDevicePresence` + `CompanionDeviceService` (API 31+; permission `REQUEST_OBSERVE_COMPANION_DEVICE_PRESENCE`) binds the app when the device is in BLE range. API 36 deprecates the String overload in favour of `ObservingDevicePresenceRequest`.
  - "Companion device pairing doesn't create connections on its own."
  - **Recommended for the Pi pairing UX and for background reconnect.**
- **[V/S] `connectGatt(autoConnect=true)`.**
  - AOSP doc: "automatically connect as soon as the remote device becomes available".
  - Punch Through: in practice it does not time out and is slower, because the stack uses a background low-duty scan. `autoConnect=false` has a ~30 s direct-connect timeout (status 133/0x93 GATT_CONNECTION_TIMEOUT).
  - **[H]** Use `false` for user-initiated connects and `true` (or CDM presence) for background reconnects.
- **[S] Doze/App Standby.** A foreground service with type connectedDevice is exempt from most restrictions. Without one, an ongoing GATT connection persists in the Bluetooth process, but app callbacks can be delayed when the process is cached or frozen. **[H]** Always use the FGS while a session is open.

## 6. Security from the central side

- **[V] iOS has no pairing API.** ADG §58.10: "The accessory should not request pairing until an ATT request is rejected using the Insufficient Authentication error code… the device may proceed with the necessary security procedures." §58.9: do not require security for service discovery.
  - For the Pi: put session characteristics behind encrypt/authenticated-encrypt permissions (BlueZ `encrypt-authenticated-read/write` flags). The first access fails with Insufficient Authentication and iOS shows the system pairing dialog.
  - `CBATTError.insufficientAuthentication` exists. `CBError.encryptionTimedOut` exists (iOS 13.4+).
  - **[H] Pairing method.** iOS supports LE Secure Connections with Just Works (Pair/Cancel alert), Passkey Entry (iOS shows a PIN field, or the Pi displays a code), and Numeric Comparison. OOB is not exposed to third-party apps; ADG §46 OOB pairing is an MFi/accessory feature.
  - **[H] For a root shell, use the Pi's IO capability `DisplayOnly` or `DisplayYesNo`** (via a BlueZ agent) so the session is MITM-protected with a passkey or numeric comparison shown on the Pi's console or display. Headless Pi: consider app-layer authentication instead (see below).
- **[V] iOS hides peer addresses.** `CBPeer.identifier`: "The first time a local manager encounters a peer, the system assigns the peer a UUID". The app never sees the MAC. ADG §58.8: the iPhone uses a random (resolvable private) address, so "The accessory should be able to resolve a Resolvable Private Address" (BlueZ does this via the IRK after bonding).
  - **[H]** The Pi cannot identify a phone by address without bonding. Use an app-layer identity (key pair) for authorization.
- **[V] Android:**
  - `createBond()`: "Android system services will handle the necessary user interactions". Results come via `ACTION_BOND_STATE_CHANGED`.
  - `createBondOutOfBand`, `removeBond`, and `PAIRING_VARIANT_CONSENT`/`PASSKEY`/`OOB_CONSENT` are `@SystemApi` (hidden).
  - `setPairingConfirmation` requires `BLUETOOTH_PRIVILEGED`.
  - `setPin()` is public.
  - **Third-party apps therefore cannot do OOB or remove bonds programmatically; users must "Forget" the device in Settings.**
  - AOSP `BluetoothGatt.onCharacteristicWrite` automatically retries a write with `AUTHENTICATION_NO_MITM`, then `MITM`, on GATT_INSUFFICIENT_AUTHENTICATION/ENCRYPTION. This triggers pairing implicitly, like iOS.
  - **[S]** Punch Through: on auth failures call `createBond()` first and wait. A known Android 13 issue gives status 133 on read after confirming the bonding dialog.
- **[H] Just Works on both OSes still shows a system confirmation dialog.** iOS shows "Bluetooth Pairing Request – Pair". Android shows a consent dialog or notification for app-initiated pairing. Just Works gives encryption without MITM protection.
- **[H] Recommendation:** do not rely solely on BLE pairing for shell authorization. Run an app-layer authenticated key exchange (for example Noise_XX or SSH-style host key + client key pinning) over the GATT or CoC channel, with LE Secure Connections as defence-in-depth. This also sidesteps the iOS/Android pairing UX and hidden-API asymmetries and works over an unencrypted CoC.

## 7. MFi and App Store

- **[V] MFi is not required for BLE-only accessories using Core Bluetooth.** MFi FAQ: accessories "that use only Bluetooth Low Energy (BLE)" are outside the program, except BLE HomeKit and MFi hearing aids. "Who should join" lists developers of accessories that connect using only BLE, Core Bluetooth, or standard iOS-supported Bluetooth profiles as not needing to join. ADG §3.8: some features need MFi, but CoreBluetooth GATT and L2CAP do not. **Classic BR/EDR SPP-style data would need MFi**, which is another reason BLE is the only option.
- **[V] Guideline 2.5.2** forbids downloading or executing code that "introduces or changes features or functionality of the app". **[H]** A terminal client executes nothing locally: the shell runs on the Pi, the same model as SSH clients. Termius, Blink Shell, and Prompt are published on the App Store (well-known, not re-verified today).
  - Guideline 4.2.7 (Remote Desktop Clients) targets mirroring apps and should not apply to a text terminal.
  - Requirements: `NSBluetoothAlwaysUsageDescription` purpose string (5.1.1(ii)), and justification of the `bluetooth-central` background mode in review notes.

---

## Constraints table (iOS vs Android)

| Constraint | iOS (CoreBluetooth) | Android (android.bluetooth) |
|---|---|---|
| ATT MTU control | None. OS picks it (observed 185/247/527, iOS-version dependent). Read `maximumWriteValueLength(.withoutResponse)` [V/S] | `requestMtu()`. Android 14+ always 517 on first call, later calls ignored [V] |
| Max attribute write | 512 (long write with response) [V-doc value] | 512 (`GATT_MAX_ATTR_LEN`), throws above [V] |
| DLE (251) | Automatic, iPhone 7+ (WWDC17). Peripheral must do LL length update before MTU (ADG §58.11) [V] | Automatic, device dependent [S] |
| 2M PHY | No API. Peripheral initiates [S/H] | `setPreferredPhy`/`readPhy`, `isLe2MPhySupported()` API 26 [V] |
| Connection interval | Peripheral must request it. Min 15 ms, multiple of 15. 11.25 ms only with HID. iOS may counter-offer [V ADG §58.6] | `requestConnectionPriority`: HIGH 11.25–15 ms, BALANCED 30–50 ms, LOW_POWER 100–125 ms (AOSP config.xml, OEM-overridable) [V] |
| Typical app throughput | ~30–100 KB/s. WWDC17 best case ~394 kbps with L2CAP+DLE+15 ms. iOS uses ~50% of the CI [V/S] | ~50–100 KB/s, high variance across OEMs [S] |
| WNR flow control | `canSendWriteWithoutResponse` + `peripheralIsReady(toSendWriteWithoutResponse:)` (iOS 11+) [V] | One write in flight. Wait for `onCharacteristicWrite` (held back while congested). `ERROR_GATT_WRITE_REQUEST_BUSY`=201 (API 33) [V AOSP] |
| GATT op concurrency | Internally queued by CoreBluetooth [H] | One outstanding op per BluetoothGatt (`mDeviceBusy`) and per remote address across apps (`mPermits`) [V AOSP] |
| Notification backpressure | None at API level; app must buffer [V/H] | None. Callback on binder thread or Handler [V AOSP] |
| L2CAP CoC | iOS 11+ `openL2CAPChannel(PSM)`. Streams with `hasSpaceAvailable`. No encryption flag (server decides). PSM via `ABDD3056-…` characteristic. Multi-channel bug. Params not tunable [V/S] | API 29+ `createL2capChannel` (auth+enc, needs bond) / `createInsecureL2capChannel`. Blocking streams. `getMaxTransmitPacketSize()` [V] |
| CoC ↔ BlueZ interop | No verified public report. Plausible. Prototype first [S/H] | Same [S/H] |
| Background | `bluetooth-central` mode. Notifications wake app (~10 s per wake). State restoration. No relaunch after force-quit or BT toggle (QA1962). CoC-in-background undocumented [V/H] | FGS type `connectedDevice` (+`FOREGROUND_SERVICE_CONNECTED_DEVICE`) when targeting 14+. CDM presence + background-FGS-start permissions [V] |
| Reconnect | `connect()` never times out (pending connect). iOS 17 `EnableAutoReconnect` [V] | `autoConnect=true` (slow background connect, no timeout) or CDM presence [V/S] |
| Permissions | `NSBluetoothAlwaysUsageDescription`. Bluetooth authorization prompt [V-known] | `BLUETOOTH_SCAN` (neverForLocation), `BLUETOOTH_CONNECT` (API 31+). Location on ≤ 30 [V] |
| Pairing initiation | No API. Triggered by Insufficient Authentication/Encryption ATT error [V ADG §58.10] | `createBond()` public, or implicit via auth error retry in BluetoothGatt [V AOSP] |
| Pairing methods for apps | LESC Just Works/Passkey/Numeric Comparison via system UI. No OOB for apps [H] | Same via system UI. OOB / consent / removeBond are `@SystemApi` [V AOSP] |
| Peer identity | Random UUID per local device (`CBPeer.identifier`). No MAC. Phone uses RPA [V] | MAC (or RPA identity after bond) visible [V-known] |
| Unbond from app | Impossible (user: Settings > Forget) [H] | `removeBond` hidden. User must forget the device [V] |
| MFi / store | MFi not needed for BLE-only. 2.5.2 not triggered for a remote shell [V/H] | n/a (Play policy: FGS type declaration and justification) [H] |

## Design implications (hypotheses for the architecture study)
1. **Baseline transport = GATT.** Use one NUS-like service:
   - RX: Write Without Response.
   - TX: Notify.
   - Control: Write With Response, Indicate.
   - PSM: read-only characteristic with Apple's PSM UUID.
2. **App-level credit window** in both directions. Required because notifications have no backpressure, and it makes the PTY reader on the Pi block naturally.
3. **Optional CoC upgrade** negotiated over GATT. Use an unencrypted BlueZ CoC (`BT_SECURITY_LOW`) protected by the app-layer AEAD. Fall back to GATT on any failure or timeout. Use one channel only (iOS multi-channel bug).
4. **Pi side:** request 15 ms CI (iOS) and let Android request HIGH. Do the DLE update before the MTU exchange, offer MTU 517, and request 2M PHY. Disable the BlueZ battery plugin and set `ControllerMode=le`.
5. **Session persistence on the Pi:** a detached PTY session survives BLE drops, iOS force-quit, and Android process death. Resync the screen on reconnect, so reconnect logic is uniform.
6. **App-layer auth (key pinning):** iOS/Android pairing asymmetries and hidden APIs make BLE pairing alone unsuitable as the authorization mechanism for a root-capable shell.

## Open items to validate on hardware
- iOS → BlueZ (kernel ≥ 5.10) CoC open, with and without encryption, and whether iOS triggers pairing on a CoC security reject.
- iOS buffering before credits are withheld.
- Whether CoC data is delivered while the app is backgrounded but not suspended.
- Actual iOS MTU on current iPhones / iOS 18–26 against a BlueZ server offering 517.
- Android OEM variance (Samsung, Xiaomi) for CoC and `CONNECTION_PRIORITY_HIGH`.
