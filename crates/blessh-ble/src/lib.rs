//! Platform Bluetooth layer shared by the server and the client: what each
//! stack needs beyond the sans-IO core in `blessh-core`. Linux uses BlueZ
//! through bluer, macOS CoreBluetooth through objc2.

#[cfg(target_os = "linux")]
pub mod bluez;
#[cfg(target_os = "macos")]
pub mod corebluetooth;
