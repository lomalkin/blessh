//! blessh: SSH over Bluetooth Low Energy, client and server in one program.
//!
//! `blessh [ssh options] [user@]server [command]` runs the system ssh with
//! a ProxyCommand of `blessh proxy`, which relays the session over BLE;
//! without a server it lets the user pick one (`pick`); `blessh scan` lists
//! servers in range; `blessh server ...` is the server (crate
//! blessh-server). See `cli::USAGE`.

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod cache;
mod cli;
#[cfg(target_os = "linux")]
mod linux;
mod listing;
#[cfg(target_os = "macos")]
mod macos;
mod pick;
#[cfg(target_os = "macos")]
mod responsible;

use std::{io::IsTerminal, os::unix::process::CommandExt};

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

/// Commands that only print end quietly when their reader goes away
/// (`blessh scan | head`), like other command-line tools; Rust ignores
/// SIGPIPE, which turns that into a panic. Not for the proxy and the
/// server, which write to sockets and must see the error instead.
fn default_sigpipe() {
    // SAFETY: restores the default disposition before any thread starts.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
}

/// Ends this process when its parent goes away: a scan that a list
/// started (`pick`) must not outlive it, even if it was killed. On macOS
/// this is the scan's first process, whose child follows it
/// (`responsible`).
fn exit_with_parent() {
    // SAFETY: no preconditions.
    let parent = unsafe { libc::getppid() };
    let _ = std::thread::Builder::new()
        .name("blessh-parent".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            // SAFETY: no preconditions.
            if unsafe { libc::getppid() } != parent {
                std::process::exit(1);
            }
        });
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let first = args.get(1).map(String::as_str);
    let server =
        first == Some("server") && args.get(2).map_or(true, |c| blessh_server::is_command(c));
    let printing = match first {
        Some("server") => !matches!(args.get(2).map(String::as_str), Some("run" | "session")),
        Some("scan" | "-h" | "--help" | "help" | "-V" | "--version") | None => true,
        _ => false,
    };
    if printing {
        default_sigpipe();
    }
    if first == Some("scan") {
        exit_with_parent();
    }
    // macOS asks about Bluetooth for the responsible process; these
    // commands use or check the permission as blessh itself.
    #[cfg(target_os = "macos")]
    if matches!(first, Some("scan" | "proxy"))
        || server
            && matches!(
                args.get(2).map(String::as_str),
                Some("enable" | "disable" | "status")
            )
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
        None if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) => {
            let e = std::process::Command::new(std::env::current_exe().unwrap_or("blessh".into()))
                .arg("scan")
                .exec();
            eprintln!("blessh: cannot run blessh scan: {e}");
            1
        }
        None => pick::run(&[]),
        Some("-h") | Some("--help") | Some("help") => {
            println!("{}", cli::USAGE);
            0
        }
        Some("-V") | Some("--version") => {
            println!("blessh {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some(_) if !cli::names_server(&args[1..]) => pick::run(&args[1..]),
        Some(_) => {
            let e = cli::exec_ssh(&args[1..], None);
            eprintln!("blessh: cannot run ssh: {e}");
            1
        }
    };
    std::process::exit(code);
}
