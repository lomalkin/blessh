//! `blessh server enable|disable|status` on macOS (docs/macos-server.md).
//!
//! The server runs as a Homebrew service (`brew services`, LaunchAgent
//! sh.brew.blessh, homebrew.mxcl.blessh before Homebrew 5) in the user's
//! session. Its sessions go to the
//! launchd job org.blessh.sshd, which `enable` installs once with sudo: the
//! system's sshd on 127.0.0.1 with /etc/blessh/sshd_config, which includes
//! the system configuration. Upgrades of the formula need no sudo, since
//! nothing of blessh runs as root.
//!
//! The commands are idempotent: what is already in place is left alone,
//! and sudo is asked for only when a root-owned file has to change.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use blessh_ble::corebluetooth::{self, Permission};
use clap::Parser;

use crate::macos::{server_name, NAME_FILE, SSHD_PORT};

pub(crate) const DAEMON_PLIST: &str = "/Library/LaunchDaemons/org.blessh.sshd.plist";
const DAEMON_LABEL: &str = "org.blessh.sshd";
const SSHD_CONFIG: &str = "/etc/blessh/sshd_config";
const HOST_KEY: &str = "/etc/ssh/ssh_host_ed25519_key.pub";
const PLIST_TEMPLATE: &str = include_str!("../../../packaging/macos/org.blessh.sshd.plist");
const SSHD_CONFIG_TEXT: &str = include_str!("../../../packaging/macos/sshd_config");
/// The sshd configuration of the 0.1 tarball, replaced when unchanged.
const OLD_SSHD_CONFIG_TEXT: &str = "\
# sshd configuration for SSH sessions over Bluetooth LE on macOS.
#
# Each BLE session gets its own `sshd -i -f /etc/blessh/sshd_config` from
# the org.blessh.session launchd job. All settings, including the host
# keys, come from /etc/ssh/sshd_config, included below; Remote Login does
# not have to be on. Settings placed above the Include apply to BLE
# sessions only, for example:
#AllowGroups blessh
# BLE sessions reach sshd over a loopback TCP connection, so sshd logs them
# as coming from 127.0.0.1 and `Match Address 127.0.0.1` blocks in the
# system configuration apply to them.

Include /etc/ssh/sshd_config
";
/// The Homebrew formula and the labels its service has had.
const FORMULA: &str = "blessh";
const SERVICE_LABELS: [&str; 2] = ["sh.brew.blessh", "homebrew.mxcl.blessh"];
/// What the 0.1 release tarball installed.
const OLD_DAEMON: &str = "/Library/LaunchDaemons/org.blessh.session.plist";
const OLD_AGENT: &str = "Library/LaunchAgents/org.blessh.server.plist";
const OLD_CLIENT: &str = "/usr/local/bin/blessh";
/// How long the Bluetooth permission prompt may wait for an answer.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Parser, Debug)]
struct Args {}

pub fn main(cmd: &str, argv: Vec<String>) -> i32 {
    let _ = Args::parse_from(argv);
    // SAFETY: no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!(
            "blessh server {cmd}: run it without sudo, as the user whose session serves \
             Bluetooth; it asks for sudo itself when it has to"
        );
        return 2;
    }
    let result = match cmd {
        "enable" => enable(),
        "disable" => disable(),
        _ => return status(),
    };
    match result {
        Ok(()) => status(),
        Err(e) => {
            eprintln!("blessh server {cmd}: {e}");
            1
        }
    }
}

fn enable() -> Result<(), String> {
    let brew = brew().ok_or(
        "Homebrew not found; the server is set up through it: \
         brew install lomalkin/tap/blessh",
    )?;
    if !run_quiet(Command::new(&brew).args(["list", "--versions", FORMULA])) {
        return Err(format!(
            "blessh is not installed with Homebrew; the server is set up through it: \
             {} install lomalkin/tap/blessh",
            brew.display()
        ));
    }
    check_bluetooth()?;
    remove_old_agent();
    if Path::new(OLD_CLIENT).exists() {
        println!("Note: {OLD_CLIENT} is an older blessh; remove it with: sudo rm {OLD_CLIENT}");
    }

    let plist = PLIST_TEMPLATE.replace("@PORT@", &SSHD_PORT.to_string());
    let installed = std::fs::read_to_string(DAEMON_PLIST).ok();
    let up_to_date = installed.as_deref() == Some(plist.as_str())
        && daemon_loaded()
        && Path::new(SSHD_CONFIG).exists()
        && !old_config()
        && Path::new(HOST_KEY).exists()
        && !Path::new(OLD_DAEMON).exists();
    if !up_to_date {
        if installed.is_none() && port_taken() {
            return Err(format!(
                "127.0.0.1 port {SSHD_PORT} is used by another program; the sshd job needs it"
            ));
        }
        install_daemon(&plist)?;
    }

    println!("Starting the server with Homebrew:");
    if !Command::new(&brew)
        .args(["services", "start", FORMULA])
        .status()
        .is_ok_and(|s| s.success())
    {
        return Err(format!(
            "{} services start {FORMULA} failed",
            brew.display()
        ));
    }
    Ok(())
}

fn disable() -> Result<(), String> {
    if let Some(brew) = brew() {
        if service_loaded() {
            println!("Stopping the server with Homebrew:");
            let _ = Command::new(&brew)
                .args(["services", "stop", FORMULA])
                .status();
        }
    }
    remove_old_agent();
    if Path::new(DAEMON_PLIST).exists() || Path::new(OLD_DAEMON).exists() {
        println!("Removing the sshd job {DAEMON_LABEL} (sudo):");
        sudo_script(&format!(
            "launchctl bootout system/{DAEMON_LABEL} 2>/dev/null || true\n\
             rm -f {DAEMON_PLIST}\n{}",
            OLD_DAEMON_REMOVAL
        ))?;
    }
    println!("/etc/blessh (sshd_config, name) was kept; remove it with: sudo rm -r /etc/blessh");
    Ok(())
}

fn status() -> i32 {
    let row = |k: &str, v: &str| println!("  {k:<10} {v}");
    match server_name(None, Path::new(NAME_FILE)) {
        Ok(n) => row("Name", &format!("{n} (another one: put it in {NAME_FILE})")),
        Err(e) => row("Name", &e),
    }
    let running = service_pid();
    row(
        "Server",
        &match running {
            Some(Some(pid)) => format!("running (pid {pid}), brew service {FORMULA}"),
            Some(None) => format!("started but not running, brew service {FORMULA}"),
            None => "not started (blessh server enable)".into(),
        },
    );
    let sshd = Path::new(DAEMON_PLIST).exists() && daemon_loaded();
    row(
        "SSH",
        &if sshd {
            format!("system sshd on 127.0.0.1 port {SSHD_PORT}, configuration {SSHD_CONFIG}")
        } else {
            "the sshd job is not installed (blessh server enable)".into()
        },
    );
    row(
        "Bluetooth",
        match corebluetooth::permission() {
            Permission::Allowed => "allowed",
            Permission::Undecided => "not decided yet (blessh server enable asks)",
            Permission::Denied => {
                "denied: allow blessh in System Settings, Privacy & Security, Bluetooth"
            }
        },
    );
    if let Some(fp) = fingerprint() {
        row("Host key", &format!("{fp} (same as Remote Login)"));
    }
    if let Some(prefix) = brew().and_then(|b| b.parent()?.parent().map(Path::to_path_buf)) {
        row(
            "Log",
            &prefix.join("var/log/blessh.log").display().to_string(),
        );
    }
    if matches!(running, Some(Some(_))) && sshd {
        0
    } else {
        3
    }
}

/// The Homebrew executable: next to this program when it was installed
/// by Homebrew, else in the standard prefixes or on PATH.
fn brew() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| std::fs::canonicalize(e).ok())
        .and_then(|e| {
            e.ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == "Cellar"))
                .and_then(Path::parent)
                .map(|p| p.join("bin/brew"))
        });
    let path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("brew"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    beside
        .into_iter()
        .chain([
            PathBuf::from("/opt/homebrew/bin/brew"),
            PathBuf::from("/usr/local/bin/brew"),
        ])
        .chain(path)
        .find(|p| p.is_file())
}

/// Makes sure blessh may use Bluetooth, asking now if macOS has not
/// decided: the service could not answer the prompt in the background.
fn check_bluetooth() -> Result<(), String> {
    match corebluetooth::permission() {
        Permission::Allowed => Ok(()),
        Permission::Denied => Err(
            "blessh may not use Bluetooth: allow it in System Settings, Privacy & Security, \
             Bluetooth, then run this again"
                .into(),
        ),
        Permission::Undecided => {
            println!("macOS asks whether blessh may use Bluetooth; answer on the Mac's screen.");
            corebluetooth::ask_permission(PROMPT_TIMEOUT).map_err(|e| e.to_string())?;
            match corebluetooth::permission() {
                Permission::Allowed => Ok(()),
                _ => Err("blessh was not allowed to use Bluetooth".into()),
            }
        }
    }
}

fn run_quiet(c: &mut Command) -> bool {
    c.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn daemon_loaded() -> bool {
    run_quiet(Command::new("launchctl").args(["print", &format!("system/{DAEMON_LABEL}")]))
}

fn uid() -> u32 {
    // SAFETY: no preconditions.
    unsafe { libc::getuid() }
}

fn service_loaded() -> bool {
    service_pid().is_some()
}

/// The Homebrew service: None when not loaded, Some(pid) when loaded.
fn service_pid() -> Option<Option<u32>> {
    SERVICE_LABELS.iter().find_map(|label| {
        let out = Command::new("launchctl")
            .args(["print", &format!("gui/{}/{label}", uid())])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Some(
            text.lines()
                .find_map(|l| l.trim().strip_prefix("pid = "))
                .and_then(|p| p.trim().parse().ok()),
        )
    })
}

/// Whether the sshd configuration is the 0.1 tarball's, never edited.
fn old_config() -> bool {
    std::fs::read_to_string(SSHD_CONFIG).is_ok_and(|c| c == OLD_SSHD_CONFIG_TEXT)
}

fn port_taken() -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, SSHD_PORT));
    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
}

fn fingerprint() -> Option<String> {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f", HOST_KEY])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
}

/// The 0.1 tarball's LaunchAgent, in the user's own domain.
fn remove_old_agent() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let agent = Path::new(&home).join(OLD_AGENT);
    if agent.exists() {
        let _ = run_quiet(
            Command::new("launchctl")
                .args(["bootout", &format!("gui/{}/org.blessh.server", uid())]),
        );
        let _ = std::fs::remove_file(&agent);
        println!(
            "Removed the server of an older blessh ({}).",
            agent.display()
        );
    }
}

/// Removes the root parts of the 0.1 tarball.
const OLD_DAEMON_REMOVAL: &str = "\
if [ -e /Library/LaunchDaemons/org.blessh.session.plist ]; then
    launchctl bootout system/org.blessh.session 2>/dev/null || true
    rm -f /Library/LaunchDaemons/org.blessh.session.plist /var/run/blessh-session.sock
    rm -rf /usr/local/libexec/blessh
fi
";

fn install_daemon(plist: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("blessh-enable-{}", std::process::id()));
    let write = |dir: &Path| -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("plist"), plist)?;
        std::fs::write(dir.join("sshd_config"), SSHD_CONFIG_TEXT)
    };
    write(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let d = dir.display();
    println!(
        "Installing the sshd job {DAEMON_LABEL}: the system's sshd on 127.0.0.1 port \
         {SSHD_PORT} with {SSHD_CONFIG}, which includes /etc/ssh/sshd_config (sudo, once):"
    );
    // bootout returns before the job is gone; bootstrap fails until it is.
    let script = format!(
        "set -e\n\
         install -d -m 0755 /etc/blessh\n\
         [ -e {SSHD_CONFIG} ] || install -m 0644 {d}/sshd_config {SSHD_CONFIG}\n\
         ssh-keygen -A > /dev/null\n\
         {OLD_DAEMON_REMOVAL}\
         install -o root -g wheel -m 0644 {d}/plist {DAEMON_PLIST}\n\
         launchctl bootout system/{DAEMON_LABEL} 2>/dev/null || true\n\
         for i in 1 2 3 4 5 6 7 8 9 10; do\n\
             launchctl print system/{DAEMON_LABEL} > /dev/null 2>&1 || break\n\
             sleep 0.5\n\
         done\n\
         launchctl bootstrap system {DAEMON_PLIST}\n"
    );
    // An unedited configuration of the 0.1 tarball is replaced; any other
    // existing one is the administrator's and stays.
    let script = if old_config() {
        script.replace(
            &format!("[ -e {SSHD_CONFIG} ] ||"),
            &format!("rm -f {SSHD_CONFIG}; [ -e {SSHD_CONFIG} ] ||"),
        )
    } else {
        script
    };
    let result = sudo_script(&script);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn sudo_script(script: &str) -> Result<(), String> {
    let ok = Command::new("sudo")
        .args(["/bin/sh", "-c", script])
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        Err("the step with sudo failed".into())
    }
}
