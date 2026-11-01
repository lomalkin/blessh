//! The blessh server, run as `blessh server <command>` from the one
//! `blessh` program.
//!
//! `run` is the daemon: the platform module registers the BLE service with
//! the system Bluetooth stack and hands every session to sshd. `enable`,
//! `disable` and `status` set the daemon up the platform's way (systemd on
//! Linux, Homebrew services and a localhost sshd job on macOS). `session`
//! is the per-session helper of the Linux session socket.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_admin;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_admin;
#[cfg(target_os = "linux")]
mod session;

pub const USAGE: &str = "\
Usage: blessh server <command> [options]

Commands:
  enable    set the server up and start it, now and at every boot or login
  disable   stop the server and undo what enable set up
  status    show whether the server runs and under which name
  run       run the server in the foreground (what the service runs)

`blessh server run --help` lists the server's options.";

/// The commands of `blessh server`; any other word after `blessh server`
/// is an ssh command for a host named "server".
pub fn is_command(word: &str) -> bool {
    matches!(
        word,
        "run" | "enable" | "disable" | "status" | "session" | "help" | "-h" | "--help"
    )
}

/// Runs `blessh server <args>` and returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let Some(cmd) = args.first() else {
        println!("{USAGE}");
        return 0;
    };
    // What clap prints as the program name in help and errors.
    let argv = || -> Vec<String> {
        std::iter::once(format!("blessh server {cmd}"))
            .chain(args[1..].iter().cloned())
            .collect()
    };
    match cmd.as_str() {
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            0
        }
        "run" => run(argv()),
        "enable" | "disable" | "status" => admin(cmd, argv()),
        #[cfg(target_os = "linux")]
        "session" => session::main(argv()),
        _ => {
            eprintln!("blessh server: unknown command {cmd:?}\n\n{USAGE}");
            2
        }
    }
}

#[cfg(target_os = "linux")]
fn run(argv: Vec<String>) -> i32 {
    linux::main(argv)
}

#[cfg(target_os = "macos")]
fn run(argv: Vec<String>) -> i32 {
    macos::main(argv)
}

#[cfg(target_os = "linux")]
fn admin(cmd: &str, argv: Vec<String>) -> i32 {
    linux_admin::main(cmd, argv)
}

#[cfg(target_os = "macos")]
fn admin(cmd: &str, argv: Vec<String>) -> i32 {
    macos_admin::main(cmd, argv)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn run(_argv: Vec<String>) -> i32 {
    eprintln!("blessh server: this platform is not supported yet");
    1
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn admin(_cmd: &str, _argv: Vec<String>) -> i32 {
    run(Vec::new())
}
