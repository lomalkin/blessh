//! Platform Bluetooth layer shared by the server and the client: what each
//! stack needs beyond the sans-IO core in `blessh-core`. Linux uses BlueZ
//! through bluer; other platforms follow.

#[cfg(target_os = "linux")]
pub mod bluez;
