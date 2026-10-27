//! blessh-server: SSH sessions over Bluetooth Low Energy.
//!
//! The platform module registers the BLE service with the system Bluetooth
//! stack and hands every session to the session socket, where `sshd -i`
//! takes over.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
fn main() {
    linux::main()
}

#[cfg(target_os = "macos")]
fn main() {
    macos::main()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("blessh-server: this platform is not supported yet");
    std::process::exit(1);
}
