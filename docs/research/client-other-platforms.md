# Clients for other platforms

Reference for whoever writes a BleStream client beyond the Android app and
the Linux `ble-term-connect`. No such client is planned yet (backlog.md).
Sources, estimates and the full comparison are in the working note
[client-platforms-notes.md](client-platforms-notes.md) (2026-10-24); items marked
there as unverified need a hardware test before they are relied on.

## What every client must do

The normative rules are in [protocol.md](../protocol.md); the points that
differ between platforms:

* **Discovery.** Scan for the service UUID `5b1e7e00-…`; it is in the
  advertisement. Platforms without MAC addresses (macOS, iOS, browsers)
  identify a server by its advertised or INFO name, and its identity by the
  SSH host key.
* **Frame size.** min(ATT_MTU − 3, 500), never more: a larger write
  becomes a long write or fails. If the platform does not report the MTU,
  read INFO after connecting and use its `mtu` key (servers from 0.1.3);
  otherwise start at 20 and raise the size to the longest notification
  received.
* **One frame per ATT PDU, in order.** Serialize all GATT operations (one
  queue). Use write without response with the platform's flow control, or
  write with response, one at a time, where writes without response can be
  dropped silently: a lost frame ends the session.
* **Single ATT bearer.** v1 does not support EATT. Mobile platforms and
  browsers do not open EATT on unencrypted links; on Linux it must be off
  (`Channels = 1`).
* **Notifications.** Register the handler before enabling notifications:
  the server sends its HELLO as soon as both characteristics are in use. It
  allows 10 s for the handshake.
* **SSH.** The stream is plain SSH to OpenSSH 9.2 (Bookworm) or 10
  (Trixie): ed25519 host key, curve25519 or mlkem768x25519 key exchange
  (sntrup761 is disabled), chacha20-poly1305 or AES-GCM, password through
  keyboard-interactive or password authentication, public keys. Pin the
  host key on first use and show its fingerprint; keyboard-interactive is
  needed for PAM prompts beyond a password.

## Platforms

| Platform | Approach | Platform-specific points |
|---|---|---|
| macOS | `ProxyCommand` helper like `ble-term-connect` (CoreBluetooth); OpenSSH does the SSH part | MTU from `maximumWriteValueLength(.withoutResponse)`; gate writes on `canSendWriteWithoutResponse`; a command-line tool needs an `Info.plist` with `NSBluetoothAlwaysUsageDescription`, and the permission is attributed to the terminal app |
| Windows 10/11 | `ProxyCommand` helper (WinRT); Win32-OpenSSH supports `ProxyCommand` | MTU from `GattSession.MaxPduSize`; discover with `BluetoothCacheMode.Uncached` (Windows caches GATT databases); unpaired connections work |
| One crate for Linux, macOS, Windows | Split the BlueZ backend of `ble-term-connect` behind a transport trait | `btleplug` covers all three, if it exposes the MTU and write flow control; otherwise thin per-OS backends |
| Web (Chrome/Edge on Windows, macOS, ChromeOS, Android) | Web Bluetooth + SSH core compiled to WASM (Go `x/crypto/ssh`, or OpenSSH from Chromium's libapps) + xterm.js, served as an installable offline web app | No MTU API (use INFO `mtu`); serialize writes (the browser may reject a write while one is pending); Chrome on macOS may drop writes without response under load: use write with response there; no Safari, Firefox or iOS. The web origin can read keystrokes and keys: self-hosted, strict CSP, non-extractable or WebAuthn keys |
| iOS / iPadOS | Native app: CoreBluetooth + SSH library + SwiftTerm | swift-nio-ssh lacks keyboard-interactive (libssh2 has it); background mode `bluetooth-central`, a force-quit ends the session; a bridge to another SSH app over localhost does not work (the bridge app is suspended) |
| Android, other SSH apps | Bridge app: foreground service relaying BleStream to `127.0.0.1:<port>`, used by Termux `ssh`, JuiceSSH or Termius | Foreground service type `connectedDevice`; any local app can reach the port, host key pinning still protects the session |
| ChromeOS | Web client; the Linux container has no Bluetooth access | — |

## Server features clients can rely on

* RX accepts write without response and write with response alike.
* INFO: `proto=1;name=<name>;mtu=<ATT MTU>` (`mtu` from 0.1.3), long reads
  supported; unknown keys must be ignored.
* CLOSE 1 (busy) when another client is connected.
