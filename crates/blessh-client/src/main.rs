//! blessh: SSH over Bluetooth Low Energy, client and server in one program.
//!
//! `blessh [ssh options] [user@]server [command]` runs the system ssh with
//! a ProxyCommand of `blessh proxy`, which relays the session over BLE;
//! `blessh scan` lists servers in range; `blessh server ...` is the server
//! (crate blessh-server). See `cli::USAGE`.

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod cache;
mod cli;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod responsible;

use clap::Parser;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
    let args: Vec<String> = std::env::args().collect();
    let first = args.get(1).map(String::as_str);
    let server =
        first == Some("server") && args.get(2).map_or(true, |c| blessh_server::is_command(c));
    // macOS asks about Bluetooth for the responsible process; these
    // commands use or check the permission as blessh itself.
    #[cfg(target_os = "macos")]
    if matches!(first, Some("scan" | "proxy"))
        || server && matches!(args.get(2).map(String::as_str), Some("enable" | "status"))
    {
        if let Some(code) = responsible::respawn() {
            std::process::exit(code);
        }
    }
    if server {
        std::process::exit(blessh_server::main(&args[2..]));
    }
    blessh_core::logging::init("warn");
    let code = match first {
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
