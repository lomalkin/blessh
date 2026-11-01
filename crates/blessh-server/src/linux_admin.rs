//! `blessh server enable|disable|status` on Linux: the blessh package
//! installs the systemd units turned off; `enable` turns them on, and
//! `status` checks what the server depends on (BlueZ, the adapter, sshd)
//! and shows what it serves (docs/linux.md).
//!
//! `enable` and `disable` run themselves again through sudo when not
//! root; `status` runs the checks that need root (sshd's view of its
//! configuration) only as root and says so otherwise.

use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};

use clap::Parser;

use crate::report::{check, info, Level};

const UNITS: [&str; 2] = ["blessh-session.socket", "blessh-server.service"];
const UNIT_FILE: &str = "/usr/lib/systemd/system/blessh-server.service";
const DEFAULTS: &str = "/etc/default/blessh";
const SSHD: &str = "/usr/sbin/sshd";
const SSHD_CONFIG: &str = "/etc/blessh/sshd_config";
const HOST_KEY: &str = "/etc/ssh/ssh_host_ed25519_key.pub";
const MAIN_CONF: &str = "/etc/bluetooth/main.conf";
const ECRED: &str = "/sys/module/bluetooth/parameters/enable_ecred";

#[derive(Parser, Debug)]
struct Args {}

pub fn main(cmd: &str, argv: Vec<String>) -> i32 {
    let _ = Args::parse_from(argv);
    if !Path::new(UNIT_FILE).exists() {
        eprintln!(
            "blessh server: the server's systemd units are not installed; install the blessh package"
        );
        return 1;
    }
    match cmd {
        "enable" => {
            if !Path::new(SSHD).exists() {
                eprintln!("{}", NO_SSHD.trim_end());
                return 1;
            }
            as_root(cmd).unwrap_or_else(|| systemctl(&["enable", "--now"]))
        }
        "disable" => as_root(cmd).unwrap_or_else(|| systemctl(&["disable", "--now"])),
        _ => status(),
    }
}

/// Why `enable` refuses without sshd, and what to do.
pub const NO_SSHD: &str = "\
blessh server: the server needs OpenSSH's server, which is not installed.
  Install it:  sudo apt install openssh-server
  (it also turns on network SSH; BLE does not need it: sudo systemctl disable --now ssh)
  Then: blessh server enable
";

fn is_root() -> bool {
    // SAFETY: no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// Runs `blessh server <cmd>` again through sudo unless root; Some(code)
/// if it could not.
fn as_root(cmd: &str) -> Option<i32> {
    if is_root() {
        return None;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| "/usr/bin/blessh".into());
    let e = Command::new("sudo").arg(exe).args(["server", cmd]).exec();
    eprintln!("blessh server: cannot run sudo: {e}");
    Some(1)
}

/// Runs systemctl on both units (as root), then shows the status.
fn systemctl(verb: &[&str]) -> i32 {
    match Command::new("systemctl").args(verb).args(UNITS).status() {
        Ok(s) if s.success() => {
            status();
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("blessh server: cannot run systemctl: {e}");
            1
        }
    }
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn unit_state(verb: &str, unit: &str) -> String {
    output("systemctl", &[verb, unit]).unwrap_or_else(|| "unknown".into())
}

/// BlueZ's version as (major, minor), from `bluetoothctl --version`
/// ("bluetoothctl: 5.82").
fn parse_bluez(text: &str) -> Option<(u32, u32)> {
    let v = text.split_whitespace().last()?;
    let (major, minor) = v.split_once('.')?;
    let minor: String = minor.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// The `Channels` setting in bluetoothd's main.conf (the last one wins).
fn parse_channels(main_conf: &str) -> Option<u32> {
    main_conf
        .lines()
        .filter_map(|l| {
            let (k, v) = l.trim().split_once('=')?;
            (k.trim() == "Channels").then(|| v.trim().parse().ok())?
        })
        .next_back()
}

/// The server's options from /etc/default/blessh (`BLESSH_ARGS="..."`).
fn server_args(defaults: &str) -> Vec<String> {
    defaults
        .lines()
        .filter_map(|l| l.trim().strip_prefix("BLESSH_ARGS="))
        .next_back()
        .map(|a| {
            a.trim()
                .trim_matches('"')
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn option_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

/// A keyword's value in `sshd -T` output.
fn sshd_value<'a>(dump: &'a str, key: &str) -> Option<&'a str> {
    dump.lines().find_map(|l| {
        let (k, v) = l.split_once(' ')?;
        (k == key).then_some(v)
    })
}

/// The login policy BLE sessions get, from `sshd -T`.
fn policy(dump: &str) -> String {
    let yes = |k| sshd_value(dump, k) == Some("yes");
    let pw = if yes("passwordauthentication") || yes("kbdinteractiveauthentication") {
        "yes"
    } else {
        "no"
    };
    let mut p = format!(
        "as network SSH: passwords {pw}, root {}",
        sshd_value(dump, "permitrootlogin").unwrap_or("?")
    );
    for (key, name) in [
        ("allowusers", "AllowUsers"),
        ("allowgroups", "AllowGroups"),
        ("denyusers", "DenyUsers"),
        ("denygroups", "DenyGroups"),
    ] {
        if let Some(v) = sshd_value(dump, key) {
            p.push_str(&format!(", {name} {v}"));
        }
    }
    p
}

/// BlueZ before 5.76 shares a characteristic's sockets between devices
/// and notifies every subscriber (docs/SECURITY.md, S-2).
fn check_bluez() -> Option<(u32, u32)> {
    let text = output("bluetoothctl", &["--version"]).unwrap_or_default();
    let v = parse_bluez(&text);
    match v {
        Some((ma, mi)) if (ma, mi) < (5, 76) => check(
            "BlueZ",
            &format!("{ma}.{mi}"),
            Level::Warn,
            "nearby devices can break sessions (fix: 5.76)",
        ),
        Some((ma, mi)) => check("BlueZ", &format!("{ma}.{mi}"), Level::Ok, ""),
        None => check(
            "BlueZ",
            "unknown",
            Level::Warn,
            "bluetoothctl --version failed",
        ),
    }
    v
}

/// BlueZ before 5.67 opens EATT, which makes phones ask to pair; the
/// service's eatt-off turns it off unless main.conf sets Channels
/// (docs/linux.md, "Bookworm").
fn check_eatt(bluez: Option<(u32, u32)>) {
    if bluez.map_or(true, |v| v >= (5, 67)) {
        return;
    }
    let channels = parse_channels(&std::fs::read_to_string(MAIN_CONF).unwrap_or_default());
    let ecred = std::fs::read_to_string(ECRED).unwrap_or_default();
    match channels {
        Some(1) => check("EATT", "off (Channels = 1)", Level::Ok, ""),
        Some(n) => check(
            "EATT",
            &format!("on (Channels = {n})"),
            Level::Warn,
            "phones are asked to pair",
        ),
        None if ecred.trim() == "Y" => check(
            "EATT",
            "on",
            Level::Note,
            "the server turns it off when it starts",
        ),
        None => check("EATT", "off (kernel ECRED)", Level::Ok, ""),
    }
}

fn check_bluetooth_service() {
    if unit_state("is-enabled", "bluetooth.service") != "enabled" {
        check(
            "bluetooth.service",
            "disabled",
            Level::Fail,
            "sudo systemctl enable --now bluetooth",
        );
    } else if unit_state("is-active", "bluetooth.service") == "active" {
        check("bluetooth.service", "enabled, running", Level::Ok, "");
    } else {
        check(
            "bluetooth.service",
            "enabled, stopped",
            Level::Warn,
            "blessh starts it on demand",
        );
    }
}

/// Whether an adapter is usable. The rfkill state is left alone: a block
/// is often deliberate.
fn check_adapters() -> bool {
    let mut usable = false;
    let mut found = false;
    let mut entries: Vec<_> = std::fs::read_dir("/sys/class/bluetooth")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| !n.to_string_lossy().contains(':'))
        })
        .collect();
    entries.sort();
    for dir in entries {
        found = true;
        let hci = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let what = format!("Adapter {hci}");
        let rfkill = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("rfkill"))
            });
        let flag = |name: &str| {
            rfkill
                .as_ref()
                .and_then(|r| std::fs::read_to_string(r.join(name)).ok())
                .is_some_and(|v| v.trim() == "1")
        };
        if flag("hard") {
            check(
                &what,
                "blocked (hardware switch)",
                Level::Fail,
                "hardware switch or firmware",
            );
            continue;
        }
        if flag("soft") {
            check(
                &what,
                "blocked (rfkill)",
                Level::Fail,
                "sudo rfkill unblock bluetooth",
            );
            continue;
        }
        let path = format!("/org/bluez/{hci}");
        let powered = output(
            "timeout",
            &[
                "3",
                "busctl",
                "--system",
                "get-property",
                "org.bluez",
                &path,
                "org.bluez.Adapter1",
                "Powered",
            ],
        )
        .unwrap_or_default();
        match powered.as_str() {
            "b true" => {
                check(&what, "on", Level::Ok, "");
                usable = true;
            }
            "b false" => check(&what, "off", Level::Warn, "bluetoothctl power on"),
            _ => {
                check(&what, "present", Level::Ok, "");
                usable = true;
            }
        }
    }
    if !found {
        check("Adapter", "none found", Level::Warn, "blessh waits for one");
    }
    usable
}

/// sshd's view of the BLE configuration: whether it accepts it, and its
/// dump for the login policy. Needs root (sshd reads the host keys).
fn check_sshd() -> Option<String> {
    if !Path::new(SSHD).exists() {
        check(
            "sshd",
            "not installed",
            Level::Fail,
            "sudo apt install openssh-server",
        );
        return None;
    }
    if !is_root() {
        check(
            "sshd configuration",
            "not checked",
            Level::Note,
            "sudo blessh server status",
        );
        return None;
    }
    // sshd -t wants its privilege separation directory, which only
    // ssh.service or a session creates.
    if !Path::new("/run/sshd").exists() {
        let _ = std::fs::create_dir("/run/sshd");
    }
    let ok = Command::new(SSHD)
        .args(["-t", "-f", SSHD_CONFIG])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        check(
            "sshd configuration",
            "rejected",
            Level::Fail,
            &format!("sudo sshd -t -f {SSHD_CONFIG}"),
        );
        return None;
    }
    check("sshd configuration", "valid", Level::Ok, "");
    output(SSHD, &["-T", "-f", SSHD_CONFIG])
}

fn check_units(adapter_ok: bool) -> bool {
    let enabled = unit_state("is-enabled", "blessh-server.service");
    let active = unit_state("is-active", "blessh-server.service") == "active";
    let sockets = unit_state("is-active", "blessh-session.socket") == "active";
    match (enabled.as_str(), active) {
        ("enabled", true) if !adapter_ok => check(
            "Server",
            "running, waiting",
            Level::Warn,
            "starts serving once the adapter is on",
        ),
        ("enabled", true) => check("Server", "running", Level::Ok, ""),
        ("enabled", false) => check(
            "Server",
            "not running",
            Level::Fail,
            "journalctl -u blessh-server",
        ),
        (_, true) => check(
            "Server",
            "running, not at boot",
            Level::Note,
            "blessh server enable",
        ),
        _ => check("Server", "off", Level::Note, "blessh server enable"),
    }
    if active && !sockets {
        check(
            "Sessions",
            "socket not listening",
            Level::Fail,
            "journalctl -u blessh-session.socket",
        );
    }
    active && sockets
}

fn status() -> i32 {
    let args = server_args(&std::fs::read_to_string(DEFAULTS).unwrap_or_default());
    let bluez = check_bluez();
    check_eatt(bluez);
    check_bluetooth_service();
    let adapter_ok = check_adapters();
    let dump = check_sshd();
    let running = check_units(adapter_ok);
    if args.iter().any(|a| a == "--discoverable") {
        check("Visible", "also in device lists", Level::Ok, "");
    } else {
        check("Visible", "to blessh clients only", Level::Ok, "");
    }
    println!();
    match crate::linux::server_name(option_value(&args, "--name")) {
        Ok(n) => info(
            "Name",
            &format!("{n} (static host name, or --name in {DEFAULTS})"),
        ),
        Err(e) => info("Name", &e),
    }
    if let Some(fp) = crate::report::fingerprint(HOST_KEY) {
        info("Host key", &format!("{fp} (same as network SSH)"));
    }
    if let Some(d) = &dump {
        info("Policy", &policy(d));
    }
    info("Override", SSHD_CONFIG);
    info(
        "Options",
        &format!("BLESSH_ARGS in {DEFAULTS}, then sudo systemctl restart blessh-server"),
    );
    info("Logs", "journalctl -u blessh-server -u 'blessh-session@*'");
    if running {
        0
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluez_versions() {
        assert_eq!(parse_bluez("bluetoothctl: 5.82"), Some((5, 82)));
        assert_eq!(parse_bluez("bluetoothctl: 5.66"), Some((5, 66)));
        assert_eq!(parse_bluez("5.72-0ubuntu5"), Some((5, 72)));
        assert_eq!(parse_bluez(""), None);
        assert!((5, 66) < (5, 67) && (5, 100) > (5, 76));
    }

    #[test]
    fn channels() {
        assert_eq!(parse_channels("[GATT]\n#Channels = 3\n"), None);
        assert_eq!(parse_channels("[GATT]\nChannels = 1\n"), Some(1));
        assert_eq!(
            parse_channels("[GATT]\n  Channels=3\nChannels = 2"),
            Some(2)
        );
    }

    #[test]
    fn defaults_file() {
        let a = server_args("# x\nBLESSH_ARGS=\"--name my-pi --discoverable\"\n");
        assert_eq!(a, vec!["--name", "my-pi", "--discoverable"]);
        assert_eq!(option_value(&a, "--name"), Some("my-pi"));
        assert!(server_args("BLESSH_ARGS=\"\"\n").is_empty());
        assert!(server_args("#BLESSH_ARGS=\"--name x\"\n").is_empty());
    }

    #[test]
    fn login_policy() {
        let dump = "permitrootlogin without-password\npasswordauthentication no\n\
                    kbdinteractiveauthentication no\nallowusers alice bob\n";
        assert_eq!(
            policy(dump),
            "as network SSH: passwords no, root without-password, AllowUsers alice bob"
        );
        assert_eq!(
            policy("permitrootlogin no\nkbdinteractiveauthentication yes\n"),
            "as network SSH: passwords yes, root no"
        );
    }
}
