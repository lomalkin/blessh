//! blessh-session: started by blessh-session@.service with the session
//! socket on stdin/stdout. Reads the preamble line written by the daemon,
//! logs the session id and BLE device (so the journal of this unit ties the
//! sshd log lines to a device), then replaces itself with the given command,
//! normally `sshd -i`.
//!
//! With `--loopback` (macOS) the command gets a TCP connection over the
//! loopback interface instead of the session socket, and this program relays
//! between the two: macOS's sshd sets up a BSM audit session with the
//! client's address, fails without one ("Could not create new audit
//! session") and ends the session right after login.

use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::{
        fd::{FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Command, Stdio},
};

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
    /// Give the command a loopback TCP connection and relay to it (for
    /// sshd on macOS).
    #[arg(long)]
    loopback: bool,
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
    if args.loopback {
        match run_over_loopback(&args.command) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                error!("session {}: {e}", p.id);
                std::process::exit(1);
            }
        }
    }
    let e = Command::new(&args.command[0])
        .args(&args.command[1..])
        .exec();
    error!("cannot run {}: {e}", args.command[0]);
    std::process::exit(1);
}

/// A connected pair of loopback TCP sockets. The listener lives only until
/// the accept; a connection from another process in that window is
/// recognised by its address and refused.
fn loopback_pair() -> std::io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let ours = TcpStream::connect(listener.local_addr()?)?;
    let expected = ours.local_addr()?;
    loop {
        let (theirs, from) = listener.accept()?;
        if from == expected {
            return Ok((ours, theirs));
        }
        drop(theirs);
    }
}

/// Copies `from` to `to` until EOF, then closes the write side of `to`.
fn pump(mut from: impl Read, mut to: impl Write, close: impl FnOnce()) {
    let _ = std::io::copy(&mut from, &mut to);
    let _ = to.flush();
    close();
}

fn run_over_loopback(command: &[String]) -> Result<i32, String> {
    let (ours, theirs) = loopback_pair().map_err(|e| format!("loopback connection: {e}"))?;
    let child_out = theirs.try_clone().map_err(|e| e.to_string())?;
    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::from(OwnedFd::from(child_out)))
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", command[0]))?;
    // SAFETY: fds 0 and 1 are the session socket, owned by this process;
    // nothing else uses them from here on.
    let (stdin, stdout) = unsafe {
        (
            std::fs::File::from_raw_fd(libc::dup(0)),
            std::fs::File::from_raw_fd(libc::dup(1)),
        )
    };
    let (to_child, from_child) = (
        ours.try_clone().map_err(|e| e.to_string())?,
        ours.try_clone().map_err(|e| e.to_string())?,
    );
    let up = std::thread::spawn(move || {
        pump(stdin, &to_child, || {
            let _ = to_child.shutdown(Shutdown::Write);
        })
    });
    let down = std::thread::spawn(move || {
        pump(&from_child, stdout, || {
            // SAFETY: shuts down the write half of the session socket.
            unsafe { libc::shutdown(1, libc::SHUT_WR) };
        })
    });
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = ours.shutdown(Shutdown::Both);
    let _ = down.join();
    // The client may keep its side open; do not wait for it.
    drop(up);
    Ok(status.code().unwrap_or(1))
}
