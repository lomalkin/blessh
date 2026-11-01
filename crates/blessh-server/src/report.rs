//! The output of `blessh server status` on every platform: a table of
//! checks (what, its state, OK/WARN/FAIL/NOTE, and for anything but OK a
//! short reason or fix), then facts about the server, one per line.

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    // No check warns on macOS yet.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Warn,
    Fail,
    Note,
}

/// One line of the checks table.
pub fn check_line(what: &str, state: &str, level: Level, note: &str) -> String {
    let level = match level {
        Level::Ok => "OK",
        Level::Warn => "WARN",
        Level::Fail => "FAIL",
        Level::Note => "NOTE",
    };
    format!("  {what:<19} {state:<26} {level:<4}  {note}")
        .trim_end()
        .to_owned()
}

pub fn check(what: &str, state: &str, level: Level, note: &str) {
    println!("{}", check_line(what, state, level, note));
}

/// One line of facts after the table.
pub fn info(key: &str, value: &str) {
    println!("  {key:<9} {value}");
}

/// The fingerprint of an SSH public key file.
pub fn fingerprint(path: &str) -> Option<String> {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f", path])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(
            check_line("BlueZ", "5.82", Level::Ok, ""),
            "  BlueZ               5.82                       OK"
        );
        assert_eq!(
            check_line("Adapter hci0", "off", Level::Warn, "bluetoothctl power on"),
            "  Adapter hci0        off                        WARN  bluetoothctl power on"
        );
        // The longest lines fit 100 columns.
        assert!(
            check_line(
                "sshd configuration",
                "blocked (hardware switch)",
                Level::Fail,
                "sudo sshd -t -f /etc/blessh/sshd_config"
            )
            .len()
                <= 100
        );
    }
}
