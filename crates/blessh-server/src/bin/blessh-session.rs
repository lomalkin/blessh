//! blessh-session: started by blessh-session@.service with the session
//! socket on stdin/stdout. Reads the preamble line written by the daemon,
//! logs the session id and BLE device (so the journal of this unit ties the
//! sshd log lines to a device), then replaces itself with the given command,
//! normally `sshd -i`.

use std::{os::unix::process::CommandExt, process::Command};

use blessh_core::{
    logging,
    session::{Preamble, MAX_LEN},
};
use clap::Parser;
use log::{error, info};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Logs the BLE session preamble, then runs the session command"
)]
struct Args {
    /// Seconds to wait for the preamble.
    #[arg(long, default_value_t = 10)]
    timeout: u64,
    /// Command to run with the remaining stream on stdin/stdout.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

/// Reads one line from fd 0 a byte at a time, so nothing after the newline
/// is consumed before the command takes over the socket.
fn read_line(timeout_ms: i32) -> Result<String, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
    let mut line = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let mut pfd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) };
        if r < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("poll: {e}"));
        }
        if r == 0 {
            return Err("timed out waiting for the preamble".into());
        }
        let mut b = 0u8;
        // SAFETY: reads at most one byte into `b`.
        let n = unsafe { libc::read(0, (&mut b as *mut u8).cast(), 1) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("read: {e}"));
        }
        if n == 0 {
            return Err("connection closed before the preamble".into());
        }
        if b == b'\n' {
            return String::from_utf8(line).map_err(|_| "preamble is not UTF-8".into());
        }
        line.push(b);
        if line.len() >= MAX_LEN {
            return Err("preamble too long".into());
        }
    }
}

fn main() {
    logging::init("info");
    let args = Args::parse();
    let preamble = read_line(args.timeout.saturating_mul(1000).min(i32::MAX as u64) as i32)
        .and_then(|l| Preamble::parse(&l).map_err(|e| e.to_string()));
    let p = match preamble {
        Ok(p) => p,
        Err(e) => {
            error!("refusing session: {e}");
            std::process::exit(1);
        }
    };
    info!(
        "session {}: BLE device {} ({} address), transport {}",
        p.id, p.device, p.address_type, p.transport
    );
    let e = Command::new(&args.command[0])
        .args(&args.command[1..])
        .exec();
    error!("cannot run {}: {e}", args.command[0]);
    std::process::exit(1);
}
