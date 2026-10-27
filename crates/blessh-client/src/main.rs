//! blessh: SSH over Bluetooth Low Energy, desktop client.
//!
//! `blessh [ssh options] [user@]server [command]` runs the system ssh with
//! a ProxyCommand of `blessh proxy`, which relays the session over BLE;
//! `blessh scan` lists servers in range. See `cli::USAGE`.

mod cli;
#[cfg(target_os = "linux")]
mod linux;

use clap::Parser;

#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(not(target_os = "linux"))]
mod platform {
    use crate::cli::{ProxyArgs, ScanArgs};

    pub fn scan(_args: ScanArgs) -> i32 {
        eprintln!("blessh: this platform is not supported yet");
        1
    }

    pub fn proxy(_args: ProxyArgs) -> i32 {
        eprintln!("blessh: this platform is not supported yet");
        1
    }
}

fn main() {
    blessh_core::logging::init("warn");
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("scan") | Some("proxy") => match cli::Cmd::parse() {
            cli::Cmd::Scan(a) => platform::scan(a),
            cli::Cmd::Proxy(a) => platform::proxy(a),
        },
        Some("-h") | Some("--help") | Some("help") | None => {
            println!("{}", cli::USAGE);
            0
        }
        Some("-V") | Some("--version") => {
            println!("blessh {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some(_) => {
            let e = cli::exec_ssh(&args[1..]);
            eprintln!("blessh: cannot run ssh: {e}");
            1
        }
    };
    std::process::exit(code);
}
