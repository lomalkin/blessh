//! `blessh server enable|disable|status` on Linux: the blessh-server
//! package installs and starts the systemd units; these commands turn them
//! on and off and show their state, asking for sudo when not root.

use std::process::Command;

use clap::Parser;

const UNITS: [&str; 2] = ["blessh-session.socket", "blessh-server.service"];
const UNIT_FILE: &str = "/usr/lib/systemd/system/blessh-server.service";

#[derive(Parser, Debug)]
struct Args {}

pub fn main(cmd: &str, argv: Vec<String>) -> i32 {
    let _ = Args::parse_from(argv);
    if !std::path::Path::new(UNIT_FILE).exists() {
        eprintln!("blessh server: the server is not installed; install the blessh-server package");
        return 1;
    }
    match cmd {
        "enable" => systemctl(&["enable", "--now"]),
        "disable" => systemctl(&["disable", "--now"]),
        _ => status(),
    }
}

/// Runs systemctl on both units, through sudo unless root.
fn systemctl(verb: &[&str]) -> i32 {
    // SAFETY: no preconditions.
    let root = unsafe { libc::geteuid() } == 0;
    let mut c = if root {
        Command::new("systemctl")
    } else {
        let mut c = Command::new("sudo");
        c.arg("systemctl");
        c
    };
    c.args(verb).args(UNITS);
    match c.status() {
        Ok(s) if s.success() => status(),
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("blessh server: cannot run systemctl: {e}");
            1
        }
    }
}

fn state(verb: &str, unit: &str) -> String {
    Command::new("systemctl")
        .args([verb, unit])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|_| "unknown".into())
}

fn status() -> i32 {
    let default = std::fs::read_to_string("/etc/default/blessh").unwrap_or_default();
    let name_arg = default
        .lines()
        .find_map(|l| l.trim().strip_prefix("BLESSH_ARGS="))
        .and_then(|a| {
            let a = a.trim_matches('"');
            let mut w = a.split_whitespace();
            w.by_ref().find(|x| *x == "--name").and_then(|_| w.next())
        })
        .map(str::to_owned);
    match crate::linux::server_name(name_arg.as_deref()) {
        Ok(n) => println!("  {:<9} {n}", "Name"),
        Err(e) => println!("  {:<9} {e}", "Name"),
    }
    let mut running = true;
    for unit in UNITS {
        let active = state("is-active", unit);
        running &= active == "active";
        println!(
            "  {:<9} {active}, {} ({unit})",
            if unit.ends_with(".service") {
                "Server"
            } else {
                "Sessions"
            },
            state("is-enabled", unit)
        );
    }
    println!(
        "  {:<9} journalctl -u blessh-server -u 'blessh-session@*'",
        "Logs"
    );
    if running {
        0
    } else {
        3
    }
}
