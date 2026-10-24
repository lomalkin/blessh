> Рабочие заметки исследования (2026-10-24), приложение к `../ble-terminal-research.md`. Пути вида `scratchpad/repos/...` относятся к временным клонам; номера строк соответствуют указанным SHA.

# B. Terminal emulator engines and mobile clients you can reuse with a BLE byte-stream transport

Research date: 2026-10-24. Repos were shallow-cloned to `scratchpad/repos/<name>`. Line numbers refer to the SHAs below.
Labels: **[V]** = verified (source read or `gh api`); **[H]** = hypothesis or estimate.

## 0. Pinned revisions

| Repo | SHA | Last commit (clone) |
|---|---|---|
| termux/termux-app | `8629e632fcb95da272221be327db653fb24befe9` | 2026-09-26 |
| connectbot/connectbot | `0bea37739c44ac14a67245b410f701e0799022ad` | 2026-10-22 |
| connectbot/termlib | `56934dd1369dc5d3e1f842b384df989c3c642a12` | 2026-10-21 |
| migueldeicaza/SwiftTerm | `c3436be32e3dc26d5e2181b1e451d91e21e50d53` | 2026-10-21 |
| TerminalStudio/xterm.dart | `d35ba2cc72d0b89b0fd19af69d57df2c02f26a16` | 2025-06-19 |
| blinksh/blink | `a90b4423c8b7a86770c24a7eaa6c13b0a5904b18` | 2026-04-14 |
| neovim/libvterm (mirror) | `934bc2fbf21800ac3458a499df8820ca5fb45fd3` | 2025-11-20 |
| alacritty/alacritty | `542147067dd016fc5f49120ef0ba605a98a4e3a7` | 2026-10-16 |

## 1. Activity and license, from `gh api` on 2026-10-24 [V]

| Project | Stars | License (SPDX/GitHub) | Last push | Latest release | Notes |
|---|---|---|---|---|---|
| termux-app | 63.3k | NOASSERTION (mixed, see §2.1) | 2026-09-26 | v0.118.3 (2025-05-22) | 644 open issues |
| connectbot | 3.5k | Apache-2.0 | 2026-10-23 | — | Kotlin/Compose rewrite; uses termlib 0.3.14 |
| connectbot/termlib | 28 | Apache-2.0 | 2026-10-23 | 0.3.13 (2026-10-12) | Maven `org.connectbot:termlib`; libvterm via JNI |
| SwiftTerm | 1.7k | MIT | 2026-10-21 | v1.99.1 (2026-10-21) | very active |
| xterm.dart | 656 | MIT | 2025-06-19 | v4.0.0 (2024-02-27) | 109 open issues; slowing down |
| flutter_blue_plus | 1.0k | **custom "FlutterBluePlus License 1.6"** | 2026-10-03 | 2.3.14 | **for-profit use needs a commercial license; build-time telemetry ping** |
| universal_ble (Flutter) | 171 | BSD-3-Clause | 2026-10-22 | | alternative to FBP |
| blink | 7.0k | GPL-3.0 | 2026-06-29 | | hterm in WKWebView |
| xterm.js | 21.3k | MIT | 2026-10-13 | 6.0.0 (2025-12-22) | |
| hterm (chromium/hterm mirror) | 348 | BSD-3 | archived 2022 | | development continues inside the Chromium libapps tree [H] |
| neovim/libvterm | 188 | MIT | **archived** | v0.3.3 in header | README says neovim vendors vterm in-tree now; upstream is leonerd.org.uk |
| alacritty (alacritty_terminal crate) | 66k | Apache-2.0 (crate `license = "Apache-2.0"`, v0.26.1-dev) | 2026-10-16 | | |
| wezterm (termwiz) | 29k | NOASSERTION (repo is MIT per its LICENSE.md [H]) | 2026-10-19 | | |
| ghostty (libghostty-vt) | 62.6k | MIT | 2026-10-24 | | C API "signatures still changing"; no official iOS or Android target found |
| iSH | 20.6k | NOASSERTION (GPL-3.0 [H]) | 2026-10-24 | | local x86 emulator. Not a remote client. |
| a-Shell | 4.0k | BSD-3 | 2026-10-23 | | local shell, hterm UI |
| jackpal/Android-Terminal-Emulator | 3.2k | Apache-2.0 | **archived** 2022 | | ancestor of Termux emulator |
| kshivang/BossTerm (KMP) | 58 | LGPL-3.0 | 2026-10-19 | | Android and JVM only. No iOS. |
| KotlinMania/vt100-kotlin | 0 | Apache-2.0 | 2026-10 | | immature |

Closed-source apps that cannot be forked: Termius, JuiceSSH, La Terminal, Secure ShellFish. The SwiftTerm README (lines 7-9) says that La Terminal and Secure ShellFish are built on SwiftTerm [V]. Their quality shows what the engine can deliver in production.

## 2. Code-level findings

### 2.1 Termux `terminal-emulator` and `terminal-view` (Android, Java)

**License [V].** `termux-app/LICENSE.md` says: "released under GPLv3 only ... Exceptions: Terminal Emulator for Android code is used which is released under Apache 2.0 license. Check `terminal-view` and `terminal-emulator` libraries."
- The two subdirectories contain no LICENSE file and no per-file headers. The Apache-2.0 status rests only on the top-level statement.
- **`ExtraKeysView` is in `termux-shared/src/main/java/com/termux/shared/termux/extrakeys/`.** `termux-shared/LICENSE.md` says `src/main/java/com/termux/shared/termux/*` is **GPLv3 only**, so the extra-keys row is GPL.
- `app/.../TermuxTerminalViewClient.java` (key mapping, the Ctrl/Alt volume-key hacks) is GPLv3 as well.
- Conclusion: the two libs are reusable under Apache-2.0. The extra-keys UI and the session/view client glue must be rewritten.
- The libs are published through JitPack (README badge `jitpack.io/#termux/termux-app`).

**Transport coupling [V].**
- `TerminalSession` is declared `public final class TerminalSession extends TerminalOutput` (TerminalSession.java:31), so it cannot be subclassed.
  - `initializeEmulator()` (L123-) calls `JNI.createSubprocess(...)` and starts reader and writer threads on the PTY fd.
  - `updateSize()` (L103-110) calls `JNI.setPtyWindowSize(fd, rows, cols, cellW, cellH)` and then `mEmulator.resize(...)`.
  - `write()` (L178) only writes when `mShellPid > 0`.
- The emulator itself is decoupled.
  - `TerminalEmulator(TerminalOutput session, cols, rows, cellW, cellH, transcriptRows, client)` (TerminalEmulator.java:496).
  - `append(byte[] buffer, int length)` (L688) is the output-to-screen entry point.
  - `TerminalOutput` is a small abstract class (`write(byte[],int,int)`, `titleChanged`, `onCopyTextToClipboard`, `onPasteTextFromClipboard`, `onBell`, `onColorsChanged`). It carries the terminal's replies (DA, DSR, mouse reports) back to the host.
- `TerminalView` hard-codes the concrete type: `public TerminalSession mTermSession` (TerminalView.java:52), `attachSession(TerminalSession)` (L290).
  - Keys go out through `mTermSession.write/writeCodePoint` (L784, L908, L924, L1090).
  - Resize comes from `updateSize()` (L988-996), which calls `mTermSession.updateSize(cols, rows, fontW, lineSpacing)`.
- **BLE adaptation [H].** Patch the lib (Apache-2.0 allows it):
  - Turn `TerminalSession` into an abstract base or interface that keeps `write`, `updateSize`, `getEmulator`, `getTitle`, `reset` and `finishIfRunning`.
  - Add a `RemoteTerminalSession` that:
    - calls `emulator.append()` on the main thread when BLE notifications arrive (keep the `MainThreadHandler` + `ByteQueue` pattern, L337-346);
    - sends `write()` bytes to a BLE TX queue;
    - sends `updateSize()` as a control message carrying rows, cols and optionally pixel size, in place of `TIOCSWINSZ`.
  - Change the `TerminalView` field types. About 150-300 LOC of patch, 1-2 days.
  - Several third-party apps keep similar private forks.

**Keyboard and IME [V].**
- `onCreateInputConnection` (TerminalView.java:309) switches between `TYPE_NULL` and `VISIBLE_PASSWORD|NO_SUGGESTIONS` to work around IME problems (comments cite Termux issue #137 for CJK).
- `onKeyDown` (L770) merges hardware Ctrl/Alt with the client's virtual modifiers (`mClient.readControlKey()`, L791).
- `KeyHandler.getCode(...)` (KeyHandler.java:156) maps Android keycodes to xterm sequences, with application cursor and keypad modes.
- Very mature. Years of IME bug fixes went into this code.

**Compatibility [V].**
- 24-bit color (TerminalEmulator.java:412-413, 2531-2560).
- Mouse X10/1000/1002 and SGR 1006 (L199-206, `sendMouseEvent` L553).
- Alt screen 1047/1049 (L1852-1853) and bracketed paste 2004 (L488, 1879).
- OSC 52 clipboard (L2704, 2850) and iTerm2 OSC 1337 images (L2712, 2894). Sixel is in `TerminalSixel.java`.
- `WcWidth.java` is "wcwidth(3) for Unicode 15".
- Rendering is a plain Canvas (`TerminalRenderer.java`, 276 LOC): no ligatures and no complex-script shaping.

### 2.2 ConnectBot termlib (Android, Kotlin, Compose with libvterm via JNI) and the ConnectBot app

**License [V].** Apache-2.0 for termlib and the app. termlib bundles libvterm (MIT) under `lib/src/main/cpp/libvterm`. That copy is patched: Kitty and iTerm image callbacks are added in `Terminal.cpp`.

**Transport abstraction [V]. It is the cleanest of the Android options.**
- `sealed interface TerminalEmulator` (lib/src/main/java/org/connectbot/terminal/TerminalEmulator.kt:82) "has no dependency on any UI framework". It provides:
  - `writeInput(ByteArray, off, len)` (L107; impl L606), which feeds remote output into the emulator;
  - `resize(rows, cols)` (L117);
  - `dispatchKey` / `dispatchCharacter` (L122-136);
  - `pasteText` (L93).
- `TerminalEmulatorFactory.create(..., onKeyboardInput: (ByteArray)->Unit, onResize: ((TerminalDimensions)->Unit)?, ...)` (L252-275). `onKeyboardInput` delivers bytes to send to the host; `onResize` fires with cols, rows and pixel size.
- The UI is `@Composable fun Terminal(...)` (Terminal.kt:319).
- The ConnectBot app shows the wiring in `service/TerminalBridge.kt:490-530`:
  - `onKeyboardInput` and `onResize` map to `enqueueTransportOperation(WriteData | SetDimensions)`;
  - `onClipboardCopy` handles OSC 52.
- **BLE adaptation [H].** You can use termlib as a library directly, with no fork. Bytes from BLE notifications go to `writeInput`, `onKeyboardInput` goes to BLE writes, and `onResize` goes to a control message. Expect about 0.5-1 day for the glue.
- Alternatively, fork the ConnectBot app and add a transport:
  - `AbsTransport` (app/.../transport/AbsTransport.kt:37) has `connect`, `write` (L112), `flush`, `close`, `resize(cols, rows, w, h)` (L137) and `read`/`getOutputStreams`.
  - Its host model is URI/hostname based (`createHost(uri)`, `usesNetwork()`, `getLocalIpAddress()`). A `BleTransport` therefore needs a new URI scheme such as `ble://<mac>` plus host-list UI changes.
  - That route also gets you host management, an extra-keys row (`ui/components/TerminalKeyboard.kt`, Apache-2.0), notifications and a background service. [H] about 1-2 weeks.

**Keyboard and IME [V].** `KeyboardHandler.kt` has 723 LOC. `ImeInputView.kt` (831 LOC; `onCreateInputConnection` L171) offers a TYPE_NULL mode and a full-editor "compose mode".

**Compatibility [V].**
- README lists 256 colors and true color, East Asian wide characters, combining characters, complex-script shaping, inline images (iTerm2, Kitty) and OSC 52/133/9;4 (OscParser.kt:113).
- **Unicode width:** the bundled libvterm `unicode.c` uses Markus Kuhn's old wcwidth table. Emoji and new-codepoint widths may disagree with the Pi's glibc (Bookworm glibc 2.36 ≈ Unicode 15) [H]. This is a known source of misaligned cursors in vim and htop with emoji.
- **Risk:** termlib has only 28 stars and is pre-1.0 (0.3.x), so its API may churn. It is actively developed by the ConnectBot maintainers.

### 2.3 SwiftTerm (iOS and macOS, Swift, MIT)

**License [V].** MIT (copyright de Icaza; includes code derived from xterm.js).

**Transport abstraction [V]. Ideal for this project.**
- The `TerminalViewDelegate` protocol (Sources/SwiftTerm/Apple/TerminalViewDelegate.swift:14) includes:
  - `send(source:data: ArraySlice<UInt8>)` (L73), which delivers keyboard and reply bytes to the transport;
  - `sizeChanged(source:newCols:newRows:)` (L57), which you forward as a resize control message;
  - `setTerminalTitle` (L62), `bell` (L100), `clipboardCopy` (L111) and `requestOpenLink` (L95).
- Input goes in through `TerminalView.feed(byteArray:)` (Apple/AppleTerminalView.swift:1264) and the engine's `Terminal.feed(byteArray:)` / `feed(buffer:)` (Terminal.swift:7382-7402).
- The README (L193-199) says the core deliberately ships no SSH. The sample app wires swift-nio-ssh through the same delegate, so BLE needs only a delegate implementation that talks to CoreBluetooth.
- [H] Effort: 0.5-1 day for the glue.

**Keyboard and IME [V].**
- `TerminalAccessory` (iOS/iOSAccessoryView.swift:22) is a built-in input accessory bar with esc and ctrl (L205-206), arrows and similar keys.
- Full `UITextInput` with marked text (iOSTextInput.swift:72) covers CJK input and dictation.
- Hardware keyboard special keys come in through `pressesBegan` (iOSTerminalView.swift:3202).
- The Kitty keyboard protocol is supported (`KittyKeyboardEncoder.swift`).

**Compatibility [V].**
- ANSI, 256 and TrueColor; Sixel; Kitty graphics; iTerm2 images; mouse SGR 1006 (Terminal.swift:6454); alt screen 1049 (L6467); bracketed paste (L689).
- The project is tested against esctest (README L62, L263).
- `UnicodeWidthData.swift` is generated from **Unicode 17.0.0** UCD, including grapheme-break and emoji-variation data.

**Portability [V].**
- The `Embedded` trait gives a Foundation-free headless core, and there is a WASM build.
- Package.swift has `.android` in its PortableGraphics conditions (L41, L46).
- [H] In theory, the SwiftTerm core could run on Android through the Swift SDK for Android plus JNI. That would be novel and risky, so it is not recommended as the primary path.

### 2.4 xterm.dart (Flutter, MIT)

**Transport [V].**
- `Terminal.write(String data)` (lib/src/terminal.dart:227) feeds output into the emulator.
- `onOutput: void Function(String)` (L48) carries keyboard and reply output.
- `onResize(width, height, pixelW, pixelH)` (L52; fired from `resize()` L353-362).
- `paste()` emits bracketed paste (L321-323).
- **The API takes Strings, not bytes.** The BLE receive path needs a streaming UTF-8 decoder (`utf8.decoder.startChunkedConversion`). BLE chunking will split multibyte sequences across notifications [H, but certain to happen for MTU-sized chunks].

**Compatibility [V].**
- Parser handles 1006, 1049 and 2004 (lib/src/core/escape/parser.dart:1007, 1031, 1040).
- Width uses `UnicodeV11` wcwidth (utils/unicode_v11.dart:499), which is older than Termux and SwiftTerm.
- `TerminalView` has `hardwareKeyboardOnly`, `keyboardType` and `onKeyEvent` (terminal_view.dart:41-49).

**Maintenance [V].** The last release was v4.0.0 (2024-02) and the last commit 2025-06, with 109 open issues. The project is slowing down.

**BLE stack license trap [V].** The usual pairing, flutter_blue_plus, now ships under a custom license: for-profit organizations need a commercial license, and it sends build-time telemetry. Use `universal_ble` (BSD-3) or another plugin, or write a thin platform channel to native BLE.

### 2.5 Blink Shell (iOS, GPL-3.0) and hterm

**License [V].** `COPYING` is GPLv3. GPLv3 conflicts with App Store terms (the FSF position and the VLC precedent) [H, legal]. Blink ships on the App Store only because it holds its own copyright. A third-party fork distributed on the App Store is legally risky, so use Blink as reference only.

**Architecture [V].**
- Rendering uses hterm (`Resources/hterm_all.min.js`) inside WKWebView.
- The bridge: `addScriptMessageHandler:self name:@"interOp"` (Blink/Terminal/TermView.m:156). Output goes in through `evaluateJavaScript` calling `term_write(data)` / `term_write_b64` (Resources/term.js:215, 224). Resize comes back as `_postMessage('sigwinch', {cols, rows})` (term.js:120).
- This is a proven, production-scale pattern for a native transport feeding a WebView terminal, and it maps directly to a BLE design.

### 2.6 libvterm (C, MIT)

**API [V]** (include/vterm.h):
- `vterm_input_write` (L353) feeds bytes in;
- `vterm_output_set_callback` (L357) delivers bytes to send;
- `vterm_set_size` (L348);
- `vterm_keyboard_unichar/key` (L368-369) and `vterm_keyboard_start_paste` (L371).

It is headless and has no renderer. The neovim mirror is archived; upstream is LeoNerd's bzr repo, and neovim now vendors it in-tree. The header reports version 0.3.3.

**Shared-engine use [H].** The C core compiles on both the NDK and iOS. But you still need a renderer, selection, IME and accessibility per platform. termlib already supplies the Android half; nothing comparable and maintained exists for iOS.

### 2.7 Rust: alacritty_terminal, vte and termwiz; Zig: libghostty-vt

- **alacritty_terminal [V].** Apache-2.0, v0.26.1-dev. It is a mature engine, but its API is shaped around Alacritty and offers no stability promises [H]. A shared engine needs a uniffi or cbindgen layer, a per-platform renderer (Canvas on Android, CoreText or Metal on iOS) and per-platform IME code. [H] Effort: 6-10 person-weeks before reaching Termux or SwiftTerm parity.
- **termwiz (wezterm)** has similar trade-offs.
- **libghostty-vt.** Web search found only README-level claims: a zero-dependency C/Zig API, "API signatures still changing", macOS/Linux/Windows/WASM targets, and no official iOS or Android target found. It is a promising future option, not ready today.

### 2.8 WebView with xterm.js and native BLE (not Web Bluetooth)

- **Pattern [V via Blink].** Native BLE code talks to the WebView through a message bridge:
  - Android: `WebViewCompat.addWebMessageListener` / `WebMessagePort`, or `addJavascriptInterface`;
  - iOS: `WKScriptMessageHandler` plus `evaluateJavaScript` / `callAsyncJavaScript`.
  - Carry data as base64 or binary (Uint8Array via `WebMessagePort` ArrayBuffer support on newer Android WebView) [H].
- **Pros:** one engine for both platforms, the most-tested emulator (VS Code), fast WebGL renderer, xterm.js 6.0 (2025-12).
- **Cons [V]:** xterm.js has open mobile issues:
  - #5377 "Limited touch support on mobile devices";
  - #3600 "Erratic text output from typing ... Chrome on Android";
  - #3727 "Copy and paste do not work on touch devices";
  - #5108 "Inconsistent onData between iOS and Android in RN WebView".
- [H] Soft-keyboard and IME behavior, Ctrl handling, and focus or keyboard-resize quirks will be the main cost. You would build a native extra-keys bar plus a JS glue layer. Bridge latency (about 1-5 ms) is irrelevant next to BLE latency.
- [H] Effort: 2-4 weeks to reach Termux-like typing quality on both platforms.

### 2.9 Kotlin Multiplatform and React Native

- **KMP [V].** No mature KMP terminal with an iOS renderer turned up. BossTerm is LGPL-3.0 with Android and JVM only. vt100-kotlin has 0 stars. A "kmp-terminal" listed on klibs.io returns 404 on GitHub.
- **React Native.** Options are essentially xterm.js inside react-native-webview (see issue #5108), so it is the same as §2.8 with an extra layer.

## 3. Comparison table

| Engine / option | Platform | License (reuse) | Byte-in API | Byte-out API | Resize hook | xterm level (truecolor/mouse/altscr/bpaste/OSC52) | Unicode width | Keyboard/IME/extra keys | Fit for BLE |
|---|---|---|---|---|---|---|---|---|---|
| Termux terminal-emulator/-view | Android | Apache-2.0 (libs); ExtraKeysView GPLv3 | `TerminalEmulator.append` | `TerminalOutput.write` | `TerminalSession.updateSize`; patch out the JNI call | yes/yes/yes/yes/yes; sixel, iTerm images | Unicode 15 | best-in-class IME; extra keys must be rewritten | Good; needs a small patch because `TerminalSession` is final and PTY-coupled |
| ConnectBot termlib (libvterm) | Android (Compose) | Apache-2.0 + MIT | `writeInput` | `onKeyboardInput` callback | `onResize` callback | yes/yes/yes/yes/yes; Kitty, iTerm images | old Kuhn wcwidth (risk) | TYPE_NULL and compose IME; extra keys in ConnectBot app (Apache) | **Excellent; designed transport-agnostic** |
| ConnectBot app | Android | Apache-2.0 | via termlib | `AbsTransport.write` | `AbsTransport.resize` | as termlib | as termlib | full app UX | Good fork base; host model is network-centric |
| SwiftTerm | iOS/macOS (core portable) | MIT | `feed(byteArray:)` | delegate `send(data:)` | delegate `sizeChanged` | yes/yes/yes/yes/yes; sixel, Kitty; esctest | **Unicode 17** | UITextInput, accessory bar with esc/ctrl, hardware keys, Kitty kbd | **Excellent** |
| xterm.dart | Flutter (both) | MIT | `write(String)` | `onOutput(String)` | `onResize` | yes/yes/yes/yes/partial [H] | Unicode 11 | basic; DIY extra keys | OK; one codebase, but a slowing project and String API |
| xterm.js in WebView | both | MIT | `term.write(Uint8Array)` | `onData`/`onBinary` | `onResize` | best (VS Code) | Unicode 11 default + unicode-graphemes addon [H] | weak on mobile (open issues) | OK; one engine, mobile input risk |
| hterm (Blink) | both via WebView | BSD-3 (hterm) / GPL-3 (Blink) | `term_write` | `interOp` messages | `sigwinch` message | good | own wcwidth | Blink KB is GPL | reference pattern |
| libvterm | C, both | MIT | `vterm_input_write` | output callback | `vterm_set_size` | good | old | none (headless) | engine only; iOS renderer missing |
| alacritty_terminal / termwiz | Rust, both | Apache-2.0 / MIT | parser `advance` | event listener | `resize` | very good | modern | none | engine only; large FFI and renderer effort |
| libghostty-vt | Zig/C | MIT | yes | yes | yes | very good | modern | none | not ready (API unstable, no mobile target) |

## 4. Assessment

### Best Android engine
**ConnectBot termlib** (Apache-2.0, Maven Central, transport-agnostic API) is the best Android engine. Glue to BLE is a few hundred LOC [H].
- Fallback: the **Termux terminal-emulator/-view** libs (Apache-2.0). These are more battle-tested on IME and have more users. They need a small fork patch because `TerminalSession` is final and PTY-bound, and the extra-keys row must be rewritten because the Termux one is GPLv3.
- Tie-breaker [H]: if Unicode width correctness with emoji matters more than API cleanliness, choose Termux (Unicode 15), or plan to update libvterm's width tables in termlib.

### Best iOS engine
**SwiftTerm** (MIT). It has a delegate-based transport (`send` / `feed` / `sizeChanged`) and an input accessory with esc/ctrl. It is production-proven in La Terminal and Secure ShellFish, has Unicode 17 widths and passes esctest. There is no serious competitor among native options.

### Shared engine on both platforms [H]

| Option | Effort | Risk | Comment |
|---|---|---|---|
| Native per platform (termlib + SwiftTerm) | ~1-2 person-weeks for the terminal part per platform, plus the BLE layer | Low | Two engines may behave slightly differently (e.g. Unicode widths); the BLE protocol code can be shared as a C/Rust/KMP lib if wanted |
| Flutter (xterm.dart + universal_ble) | ~2-4 wk | Medium | Single UI. xterm.dart is slowing; IME quality is lower; avoid flutter_blue_plus for commercial use |
| WebView + xterm.js + native BLE bridge | ~2-4 wk | Medium-High | Best emulation, worst mobile input; Blink shows it can be made to work |
| libvterm or alacritty_terminal via FFI with own renderers | 6-10+ wk | High | Renderer, IME, selection and accessibility per platform anyway, so little gain |
| KMP / Compose Multiplatform terminal | n/a | High | No mature library exists |

**Recommendation [H]:** use native engines (termlib or Termux libs on Android, SwiftTerm on iOS) and share only the BLE framing and protocol logic: framing, flow control, resize/control messages, reconnect. Put that logic in a small portable library, for example Rust or C with uniffi, or KMP, or just two ~500-LOC implementations backed by one written spec and test vectors.

## 5. Cross-cutting implications for the BLE protocol (from the engine APIs)

- Every engine exposes resize as (cols, rows), and most also give pixel width and height (Termux, termlib, xterm.dart). The BLE control channel should carry `cols, rows, xpixel, ypixel` so it maps one-to-one onto `TIOCSWINSZ` on the Pi.
- Engines emit replies such as DA/DSR, mouse reports and OSC color queries on the same output path as keystrokes. The upstream channel must carry arbitrary bytes, not only "keys".
- Byte-oriented engines (Termux, termlib, SwiftTerm, libvterm) tolerate UTF-8 split across BLE packets. xterm.dart and some WebView paths need a streaming decoder.
- Feed output on the UI thread or the engine's lock in batches. Termux uses a 64 KB `ByteQueue` with a main-thread handler. Coalescing BLE notifications before `append`/`feed` reduces redraws.
- Unicode width mismatches between client tables and server glibc wcwidth corrupt the vim and htop layout. Pin and test emoji and CJK widths against the Pi's locale.

## 6. Key permalinks
- Termux TerminalSession (final, PTY-coupled): https://github.com/termux/termux-app/blob/8629e632fcb95da272221be327db653fb24befe9/terminal-emulator/src/main/java/com/termux/terminal/TerminalSession.java#L31-L131
- Termux TerminalOutput: https://github.com/termux/termux-app/blob/8629e632fcb95da272221be327db653fb24befe9/terminal-emulator/src/main/java/com/termux/terminal/TerminalOutput.java
- Termux TerminalEmulator.append / ctor: https://github.com/termux/termux-app/blob/8629e632fcb95da272221be327db653fb24befe9/terminal-emulator/src/main/java/com/termux/terminal/TerminalEmulator.java#L496 , #L688
- Termux TerminalView IME/keys/resize: https://github.com/termux/termux-app/blob/8629e632fcb95da272221be327db653fb24befe9/terminal-view/src/main/java/com/termux/view/TerminalView.java#L309 , #L770 , #L988-L996
- Termux LICENSE.md: https://github.com/termux/termux-app/blob/8629e632fcb95da272221be327db653fb24befe9/LICENSE.md ; termux-shared: https://github.com/termux/termux-app/blob/8629e632fcb95da272221be327db653fb24befe9/termux-shared/LICENSE.md
- termlib TerminalEmulator API: https://github.com/connectbot/termlib/blob/56934dd1369dc5d3e1f842b384df989c3c642a12/lib/src/main/java/org/connectbot/terminal/TerminalEmulator.kt#L82-L275
- ConnectBot TerminalBridge wiring: https://github.com/connectbot/connectbot/blob/0bea37739c44ac14a67245b410f701e0799022ad/app/src/main/java/org/connectbot/service/TerminalBridge.kt#L490-L530
- ConnectBot AbsTransport: https://github.com/connectbot/connectbot/blob/0bea37739c44ac14a67245b410f701e0799022ad/app/src/main/java/org/connectbot/transport/AbsTransport.kt#L37-L137
- SwiftTerm delegate: https://github.com/migueldeicaza/SwiftTerm/blob/c3436be32e3dc26d5e2181b1e451d91e21e50d53/Sources/SwiftTerm/Apple/TerminalViewDelegate.swift#L14-L111
- SwiftTerm feed: https://github.com/migueldeicaza/SwiftTerm/blob/c3436be32e3dc26d5e2181b1e451d91e21e50d53/Sources/SwiftTerm/Apple/AppleTerminalView.swift#L1264
- SwiftTerm accessory: https://github.com/migueldeicaza/SwiftTerm/blob/c3436be32e3dc26d5e2181b1e451d91e21e50d53/Sources/SwiftTerm/iOS/iOSAccessoryView.swift#L22
- xterm.dart Terminal: https://github.com/TerminalStudio/xterm.dart/blob/d35ba2cc72d0b89b0fd19af69d57df2c02f26a16/lib/src/terminal.dart#L48-L362
- Blink bridge: https://github.com/blinksh/blink/blob/a90b4423c8b7a86770c24a7eaa6c13b0a5904b18/Blink/Terminal/TermView.m#L156 , https://github.com/blinksh/blink/blob/a90b4423c8b7a86770c24a7eaa6c13b0a5904b18/Resources/term.js#L120
- libvterm API: https://github.com/neovim/libvterm/blob/934bc2fbf21800ac3458a499df8820ca5fb45fd3/include/vterm.h#L348-L371
- flutter_blue_plus license: https://github.com/chipweinberger/flutter_blue_plus/blob/master/LICENSE
- xterm.js mobile issues: https://github.com/xtermjs/xterm.js/issues/5377 , /3600 , /3727 , /5108
