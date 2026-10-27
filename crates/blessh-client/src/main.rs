//! blessh: SSH over Bluetooth Low Energy, desktop client.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() {
    linux::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("blessh: this platform is not supported yet");
    std::process::exit(1);
}
