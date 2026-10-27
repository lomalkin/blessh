# Backlog

Ideas worth doing, not scheduled yet.

## ble-term-connect

* Find servers by name when the name comes only in the scan response
  (README, "Known issue"). With the service UUID discovery filter, BlueZ
  did not record the name from the scan response; an unfiltered scan did.
  Possible fix: when the target is a name, scan without the UUID filter
  (or with `pattern`) and check both the name and the advertised UUID.

## Clients

Options for clients beyond Android and Linux, cheapest first (details:
[research/client-other-platforms.md](research/client-other-platforms.md)):

* `ProxyCommand` helpers for macOS and Windows, ideally one crate with
  `ble-term-connect`.
* Android bridge app (BLE to localhost) for any Android SSH client.
* Web client (Web Bluetooth + WASM SSH): decided against for now (origin
  trust, supply chain, unverified browser write behaviour).
* iOS app.
