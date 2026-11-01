//! Command line of `blessh`, shared by every platform.
//!
//! ```text
//! blessh [ssh options] [user@]server [command]   run ssh over BLE
//! blessh scan [options]                          list servers in range
//! blessh proxy [options] <server>                relay stdin/stdout (ProxyCommand)
//! blessh server <command>                        the server (crate blessh-server)
//! ```
//!
//! The first form is the usual one: `server` is the name a server
//! advertises or its Bluetooth address, and the rest is passed to the
//! system ssh together with a ProxyCommand that runs `blessh proxy`.

use std::{future::Future, os::unix::process::CommandExt, process::Command};

use clap::{Args, Parser};

use crate::listing::Format;

pub const USAGE: &str = "\
Usage: blessh [ssh options] [user@]server [command]
       blessh [ssh options] [user@]
       blessh scan [options]
       blessh proxy [options] <server>
       blessh server enable|disable|status|run

Runs the system ssh over Bluetooth Low Energy: `server` is the name a
blessh server advertises or its Bluetooth address (AA:BB:CC:DD:EE:FF).
Everything else is passed to ssh, together with a ProxyCommand that runs
`blessh proxy` and a HostKeyAlias for the server's host key.

Without a server, blessh lists the servers in range on a terminal and
connects to the one whose key you press (1-9, then a-z); 0, q or Esc
quits. Without a terminal, `blessh` alone is `blessh scan`.

Environment: BLESSH_PROXY_ARGS adds options to `blessh proxy` (for example
`--adapter hci1` or `--gatt`).

`blessh server` serves this machine over BLE; `blessh server --help` says
how. To reach a host named scan, proxy or server, give a user
(`user@server`) or put `--` before it.

`blessh scan --help` and `blessh proxy --help` list their options.";

#[derive(Parser, Debug)]
#[command(name = "blessh", version, disable_help_subcommand = true)]
pub enum Cmd {
    /// List servers in range.
    Scan(ScanArgs),
    /// Connect to a server and relay the session over stdin/stdout
    /// (OpenSSH ProxyCommand).
    Proxy(ProxyArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ScanArgs {
    /// Bluetooth adapter to use.
    #[arg(long)]
    pub adapter: Option<String>,
    /// Seconds to scan; 0 scans until stopped.
    #[arg(long, default_value_t = 10)]
    pub timeout: u64,
    /// Output format: human (columns), tsv or json (one object per line).
    #[arg(long, value_enum, default_value_t = Format::Human)]
    pub format: Format,
}

impl ScanArgs {
    /// How long to scan; "until stopped" is a year.
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(match self.timeout {
            0 => 365 * 24 * 3600,
            t => t,
        })
    }
}

#[derive(Args, Debug, Clone)]
pub struct ProxyArgs {
    /// Server address (AA:BB:CC:DD:EE:FF) or advertised name; omit to use
    /// the first server found.
    pub target: Option<String>,
    /// Connect to this device (address, or CoreBluetooth identifier on
    /// macOS) without looking for the server by name, and check that the
    /// server's INFO names it `target`.
    #[arg(long, value_name = "ID")]
    pub device: Option<String>,
    /// Bluetooth adapter to use.
    #[arg(long)]
    pub adapter: Option<String>,
    /// Seconds to scan for the server.
    #[arg(long, default_value_t = 15)]
    pub scan_timeout: u64,
    /// Use the GATT stream even if the server offers an L2CAP channel.
    #[arg(long)]
    pub gatt: bool,
    /// Receive MTU of the L2CAP channel, in bytes.
    #[arg(long, default_value_t = 1024)]
    pub l2cap_mtu: u16,
    /// Bytes the server echoes at the start of a channel, to see that the
    /// channel carries data both ways.
    #[arg(long, default_value_t = blessh_core::channel::DEFAULT_PROBE, hide = true)]
    pub probe: u16,
    /// Seconds the echo may stop before the channel counts as stalled and
    /// the next one is tried.
    #[arg(long, default_value_t = 3)]
    pub stall_timeout: u64,
    /// GATT stream: receive window granted to the server, in bytes.
    #[arg(long, default_value_t = 32768)]
    pub window: u32,
    /// GATT stream: upper limit for frames written to the server (default:
    /// from the ATT MTU).
    #[arg(long)]
    pub max_frame: Option<usize>,
    /// Leave the BLE link up on exit, like a phone that keeps it for a few
    /// seconds (for tests).
    #[arg(long, hide = true)]
    pub keep_link: bool,
    /// GATT stream: write frames with response instead of without (for
    /// tests: clients whose platform loses writes without response).
    #[arg(long, hide = true)]
    pub write_with_response: bool,
}

/// ssh options that take a value (OpenSSH's getopt string in ssh.c).
const SSH_OPTIONS_WITH_VALUE: &[char] = &[
    'B', 'b', 'c', 'D', 'E', 'e', 'F', 'I', 'i', 'J', 'L', 'l', 'm', 'O', 'o', 'p', 'P', 'Q', 'R',
    'S', 'W', 'w',
];

/// Index of the destination (`[user@]host`) in ssh's arguments, skipping
/// options and their values.
pub fn destination_index(args: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            return (i + 1 < args.len()).then_some(i + 1);
        }
        let Some(flags) = a.strip_prefix('-').filter(|f| !f.is_empty()) else {
            return Some(i);
        };
        let mut chars = flags.chars();
        while let Some(c) = chars.next() {
            if SSH_OPTIONS_WITH_VALUE.contains(&c) {
                if chars.as_str().is_empty() {
                    // The value is the next argument.
                    i += 1;
                }
                break;
            }
        }
        i += 1;
    }
    None
}

/// The server to connect to, from ssh's destination: `[user@]host`, also
/// as `ssh://[user@]host[:port]`.
pub fn target_of(destination: &str) -> String {
    let d = destination.strip_prefix("ssh://").unwrap_or(destination);
    let host = d.rsplit_once('@').map_or(d, |(_, h)| h);
    let host = if destination.starts_with("ssh://") {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    } else {
        host
    };
    host.to_owned()
}

fn is_address(target: &str) -> bool {
    let octets: Vec<&str> = target.split(':').collect();
    octets.len() == 6
        && octets
            .iter()
            .all(|o| o.len() == 2 && o.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// Name under which ssh stores the server's host key: the advertised name
/// or the address, so that it does not depend on the user name or on the
/// way the server was selected.
pub fn host_key_alias(target: &str) -> String {
    if is_address(target) {
        format!("blessh-{}", target.to_ascii_lowercase().replace(':', ""))
    } else {
        format!("blessh-{target}")
    }
}

/// Whether the arguments already set an ssh option, as `-oName=value`,
/// `-o Name=value` or `-o Name value` (case-insensitive, as ssh does).
fn sets_option(args: &[String], name: &str) -> bool {
    let has = |v: &str| {
        v.trim_start()
            .get(..name.len())
            .is_some_and(|k| k.eq_ignore_ascii_case(name))
            && v.trim_start()[name.len()..].starts_with(['=', ' '])
    };
    args.iter().enumerate().any(|(i, a)| {
        a.strip_prefix("-o").is_some_and(|v| {
            if v.is_empty() {
                args.get(i + 1).is_some_and(|n| has(n))
            } else {
                has(v)
            }
        })
    })
}

/// Resolves when ssh ends its ProxyCommand (SIGHUP or SIGTERM).
pub fn stop_signal() -> std::io::Result<impl Future<Output = ()>> {
    let mut hup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            _ = hup.recv() => {}
            _ = term.recv() => {}
        }
    })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Whether the arguments name a server; without one, blessh lets the user
/// pick it (`blessh`, `blessh alice@`, `blessh -l alice`).
pub fn names_server(args: &[String]) -> bool {
    destination_index(args).is_some_and(|d| !target_of(&args[d]).is_empty())
}

/// The arguments with `server` as ssh's destination: in place of an empty
/// one (`alice@` becomes `alice@server`), else appended.
pub fn with_server(args: &[String], server: &str) -> Vec<String> {
    let mut args = args.to_vec();
    match destination_index(&args) {
        Some(d) => args[d].push_str(server),
        None => args.push(server.to_owned()),
    }
    args
}

/// Runs ssh with a ProxyCommand through this program; returns only on
/// error. With `device`, the proxy connects to that device and checks that
/// it is the server named in the arguments.
pub fn exec_ssh(args: &[String], device: Option<&str>) -> std::io::Error {
    let Some(dest) = destination_index(args) else {
        return std::io::Error::other(format!("no destination given\n\n{USAGE}"));
    };
    let target = target_of(&args[dest]);
    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => "blessh".into(),
    };
    let extra = std::env::var("BLESSH_PROXY_ARGS").unwrap_or_default();
    let proxy = format!(
        "{} proxy {}{}",
        shell_quote(&exe),
        if extra.is_empty() {
            String::new()
        } else {
            format!("{extra} ")
        },
        match device {
            Some(d) => format!("--device {} {}", shell_quote(d), shell_quote(&target)),
            None => shell_quote(&target),
        }
    );
    let mut cmd = Command::new("ssh");
    cmd.args(&args[..dest]);
    cmd.arg("-o").arg(format!("ProxyCommand={proxy}"));
    if !sets_option(args, "HostKeyAlias") {
        cmd.arg("-o")
            .arg(format!("HostKeyAlias={}", host_key_alias(&target)));
    }
    // BLE is slow: compress unless told otherwise.
    if !sets_option(args, "Compression") {
        cmd.arg("-o").arg("Compression=yes");
    }
    cmd.args(&args[dest..]);
    cmd.exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_the_destination() {
        assert_eq!(destination_index(&v(&["user@pi"])), Some(0));
        assert_eq!(destination_index(&v(&["-p", "22", "pi", "ls"])), Some(2));
        assert_eq!(destination_index(&v(&["-p22", "-v", "pi"])), Some(2));
        assert_eq!(destination_index(&v(&["-vvp", "22", "pi"])), Some(2));
        assert_eq!(
            destination_index(&v(&["-o", "User=x", "-t", "pi", "-l"])),
            Some(3)
        );
        assert_eq!(destination_index(&v(&["-oUser=x", "pi"])), Some(1));
        assert_eq!(
            destination_index(&v(&["-N", "-L", "8080:h:80", "pi"])),
            Some(3)
        );
        assert_eq!(destination_index(&v(&["--", "-weird"])), Some(1));
        assert_eq!(destination_index(&v(&["-v"])), None);
        assert_eq!(destination_index(&v(&[])), None);
    }

    #[test]
    fn target_and_alias() {
        assert_eq!(target_of("alice@my-pi"), "my-pi");
        assert_eq!(target_of("my-pi"), "my-pi");
        assert_eq!(target_of("ssh://alice@my-pi:22"), "my-pi");
        assert_eq!(target_of("alice@AA:BB:CC:DD:EE:FF"), "AA:BB:CC:DD:EE:FF");
        assert_eq!(host_key_alias("my-pi"), "blessh-my-pi");
        assert_eq!(host_key_alias("AA:BB:CC:DD:EE:FF"), "blessh-aabbccddeeff");
    }

    #[test]
    fn server_to_pick() {
        assert!(names_server(&v(&["pi"])));
        assert!(names_server(&v(&["-l", "alice", "pi", "ls"])));
        assert!(!names_server(&v(&[])));
        assert!(!names_server(&v(&["alice@"])));
        assert!(!names_server(&v(&["-l", "alice"])));
        assert!(!names_server(&v(&["-L", "8080:localhost:80"])));
        assert_eq!(
            with_server(&v(&["alice@", "ls"]), "pi"),
            v(&["alice@pi", "ls"])
        );
        assert_eq!(
            with_server(&v(&["-l", "alice"]), "pi"),
            v(&["-l", "alice", "pi"])
        );
        assert_eq!(with_server(&v(&[]), "pi"), v(&["pi"]));
    }

    #[test]
    fn detects_options_already_set() {
        assert!(sets_option(&v(&["-oHostKeyAlias=x", "pi"]), "HostKeyAlias"));
        assert!(sets_option(
            &v(&["-o", "hostkeyalias x", "pi"]),
            "HostKeyAlias"
        ));
        assert!(!sets_option(&v(&["-o", "User=x", "pi"]), "HostKeyAlias"));
        assert!(!sets_option(&v(&["-oHostKeyAliasX=1"]), "HostKeyAlias"));
    }
}
