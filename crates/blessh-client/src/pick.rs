//! Picking a server: `blessh` without one lists the servers in range on
//! the terminal and runs ssh to the one whose key the user presses
//! (docs/client.md, "Choosing a server").
//!
//! This process keeps the terminal and execs ssh, as `blessh user@server`
//! does; a child `blessh scan --format tsv --timeout 0` scans. On macOS
//! that leaves ssh with the terminal's privacy permissions: the scanner
//! runs itself again as its own responsible process for Bluetooth, which a
//! process that execs ssh must not do.
//!
//! Rows are only ever appended, so the list needs no cursor control and
//! works on any terminal; the terminal is in non-canonical mode, without
//! echo and signals, until a key is pressed.

use std::{
    io::{self, IsTerminal, Read},
    mem::MaybeUninit,
    os::fd::AsRawFd,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicI32, Ordering},
        OnceLock,
    },
};

use blessh_core::name;

use crate::{
    cli,
    listing::{self, Row, Style},
};

/// Keys of the rows, in order; `q` quits.
const KEYS: &str = "123456789abcdefghijklmnoprstuvwxyz";

/// How long the bytes of an escape sequence (an arrow key) may follow its
/// first, which is also the Escape key.
const ESC_SEQUENCE_MS: i32 = 50;

/// Characters of a CoreBluetooth identifier shown in the list: enough to
/// tell servers apart, and the row fits 80 columns.
#[cfg(target_os = "macos")]
const ID_WIDTH: Option<usize> = Some(8);
#[cfg(not(target_os = "macos"))]
const ID_WIDTH: Option<usize> = None;

/// Program run instead of `blessh scan` (tests).
const SCANNER_ENV: &str = "BLESSH_SCANNER";

/// The terminal's settings before the list, restored on every way out.
static SAVED: OnceLock<libc::termios> = OnceLock::new();
/// The scanner's process id while it runs.
static SCANNER: AtomicI32 = AtomicI32::new(0);

/// The key of row `i`, if it has one.
pub fn key_of(i: usize) -> Option<char> {
    KEYS.chars().nth(i)
}

/// The row a key selects (letters in either case).
pub fn index_of(key: u8) -> Option<usize> {
    KEYS.find(key.to_ascii_lowercase() as char)
}

/// What ssh connects to for row `i`: its name when that is a valid server
/// name no other row has, so the host key and `~/.ssh/config` entries
/// are those of the name; else its device.
pub fn target(rows: &[Row], i: usize) -> &str {
    let r = &rows[i];
    let unique = !rows
        .iter()
        .enumerate()
        .any(|(j, o)| j != i && name::same(&o.name, &r.name));
    if unique && name::check(&r.name).is_ok() {
        &r.name
    } else {
        &r.id
    }
}

/// The note of a row whose name an earlier row has.
fn note(rows: &[Row], i: usize) -> Option<String> {
    let r = &rows[i];
    if r.name.is_empty() {
        return None;
    }
    let first = rows[..i]
        .iter()
        .position(|o| name::same(&o.name, &r.name))?;
    Some(match key_of(first) {
        Some(k) => format!("same name as {k}"),
        None => "duplicate name".into(),
    })
}

fn restore_terminal() {
    if let Some(t) = SAVED.get() {
        // SAFETY: restores settings read from the same descriptor.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, t) };
    }
}

fn kill_scanner() {
    let pid = SCANNER.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe; the pid is our child's.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

extern "C" fn on_signal(sig: libc::c_int) {
    restore_terminal();
    kill_scanner();
    // SAFETY: _exit is async-signal-safe.
    unsafe { libc::_exit(128 + sig) };
}

/// Non-canonical input without echo and without signals (^C arrives as a
/// key), keys typed before the list dropped; output stays as it was.
fn enter_raw_mode() -> io::Result<()> {
    let mut t = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: tcgetattr fills the struct on success.
    if unsafe { libc::tcgetattr(0, t.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: initialised by tcgetattr above.
    let saved = unsafe { t.assume_init() };
    let _ = SAVED.set(saved);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        kill_scanner();
        hook(info);
    }));
    // SAFETY: the handler only calls async-signal-safe functions.
    unsafe {
        for sig in [libc::SIGHUP, libc::SIGTERM, libc::SIGINT, libc::SIGQUIT] {
            libc::signal(
                sig,
                on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }
    let mut raw = saved;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    // SAFETY: valid settings for the terminal on descriptor 0.
    if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: no preconditions.
    unsafe { libc::tcflush(0, libc::TCIFLUSH) };
    Ok(())
}

fn spawn_scanner() -> io::Result<Child> {
    let program = match std::env::var_os(SCANNER_ENV) {
        Some(p) => p.into(),
        None => std::env::current_exe()?,
    };
    let child = Command::new(program)
        .args(["scan", "--format", "tsv", "--timeout", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    SCANNER.store(child.id() as i32, Ordering::SeqCst);
    Ok(child)
}

/// Ends the scanner and waits for it; its exit code if it had ended.
fn stop_scanner(child: &mut Child) -> Option<i32> {
    let ended = child.try_wait().ok().flatten();
    if ended.is_none() {
        kill_scanner();
    }
    let status = ended.or_else(|| child.wait().ok());
    SCANNER.store(0, Ordering::SeqCst);
    ended.and(status).map(|s| s.code().unwrap_or(1))
}

/// Whether more input arrives within `ms`.
fn input_within(ms: i32) -> bool {
    let mut fd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut fd, 1, ms) > 0 }
}

/// Reads keys straight from the terminal: std's stdin buffers, and bytes
/// in its buffer would escape `poll`.
fn read_keys(buf: &mut [u8]) -> usize {
    // SAFETY: reads into a buffer of the given length.
    let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
    n.max(0) as usize
}

/// What a key does.
enum Action {
    Connect(usize),
    Quit(i32),
}

/// The action of the keys read at once, if any: a lone Escape quits, an
/// escape sequence (arrow keys and the like) is ignored.
fn action(keys: &[u8], rows: usize) -> Option<Action> {
    for (i, &k) in keys.iter().enumerate() {
        match k {
            0x03 => return Some(Action::Quit(130)),
            0x04 | b'0' | b'q' | b'Q' => return Some(Action::Quit(0)),
            0x1b if i + 1 == keys.len() && !input_within(ESC_SEQUENCE_MS) => {
                return Some(Action::Quit(0))
            }
            0x1b => {
                // The rest belongs to the sequence.
                let mut sink = [0u8; 64];
                while input_within(ESC_SEQUENCE_MS) && read_keys(&mut sink) > 0 {}
                return None;
            }
            k => {
                if let Some(n) = index_of(k).filter(|n| *n < rows) {
                    return Some(Action::Connect(n));
                }
            }
        }
    }
    None
}

/// Lists the servers in range, then execs ssh with `args` and the server
/// whose key was pressed; returns the exit code otherwise.
pub fn run(args: &[String]) -> i32 {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        eprintln!("blessh: no server given; choosing one from a list needs a terminal");
        return 2;
    }
    let style = Style::stdout();
    let sep = if style.arrow() == "->" { ";" } else { " ·" };
    println!(
        "{}",
        style.dim(&format!(
            "Scanning for blessh servers{sep} 1-9 a-z connect{sep} 0 q Esc quit"
        ))
    );
    if let Err(e) = enter_raw_mode() {
        eprintln!("blessh: cannot set up the terminal: {e}");
        return 1;
    }
    let mut child = match spawn_scanner() {
        Ok(c) => c,
        Err(e) => {
            restore_terminal();
            eprintln!("blessh: cannot start the scan: {e}");
            return 1;
        }
    };
    let code = pick(args, &style, &mut child);
    restore_terminal();
    code
}

fn pick(args: &[String], style: &Style, child: &mut Child) -> i32 {
    let mut out = child.stdout.take().expect("piped stdout");
    let mut pending: Vec<u8> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    loop {
        let mut fds = [
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: out.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfds.
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            stop_scanner(child);
            eprintln!("blessh: {e}");
            return 1;
        }
        if fds[1].revents != 0 {
            let mut buf = [0u8; 4096];
            match out.read(&mut buf) {
                Ok(n) if n > 0 => pending.extend_from_slice(&buf[..n]),
                _ => {
                    // The scanner ended: it said why on stderr.
                    return match stop_scanner(child) {
                        Some(0) | None => 1,
                        Some(code) => code,
                    };
                }
            }
            while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let Some(row) = listing::parse_tsv(String::from_utf8_lossy(&line).trim_end())
                else {
                    continue;
                };
                if rows.iter().any(|r| r.id == row.id) {
                    continue;
                }
                rows.push(row);
                let i = rows.len() - 1;
                let key = key_of(i).map(String::from).unwrap_or_default();
                let note = note(&rows, i);
                println!(
                    "{}",
                    listing::human(&rows[i], style, Some(&key), ID_WIDTH, note.as_deref())
                );
            }
        }
        if fds[0].revents != 0 {
            let mut keys = [0u8; 64];
            let n = read_keys(&mut keys);
            if n == 0 {
                // The terminal went away.
                stop_scanner(child);
                return 1;
            }
            match action(&keys[..n], rows.len()) {
                None => {}
                Some(Action::Quit(code)) => {
                    stop_scanner(child);
                    return code;
                }
                Some(Action::Connect(i)) => {
                    let row = &rows[i];
                    let shown = if row.name.is_empty() {
                        &row.id
                    } else {
                        &row.name
                    };
                    println!("{} {shown}", style.arrow());
                    stop_scanner(child);
                    restore_terminal();
                    let target = target(&rows, i);
                    let device = (target != row.id).then_some(row.id.as_str());
                    let e = cli::exec_ssh(&cli::with_server(args, target), device);
                    eprintln!("blessh: cannot run ssh: {e}");
                    return 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(KEYS.len(), 34);
        assert!(!KEYS.contains('q') && !KEYS.contains('0'));
        assert_eq!(key_of(0), Some('1'));
        assert_eq!(key_of(9), Some('a'));
        assert_eq!(key_of(33), Some('z'));
        assert_eq!(key_of(34), None);
        assert_eq!(index_of(b'1'), Some(0));
        assert_eq!(index_of(b'a'), Some(9));
        assert_eq!(index_of(b'A'), Some(9));
        assert_eq!(index_of(b'r'), Some(25));
        assert_eq!(index_of(b'q'), None);
        assert_eq!(index_of(b'0'), None);
        assert_eq!(index_of(b'\r'), None);
    }

    #[test]
    fn targets() {
        let rows = vec![
            Row::new("my-pi", -50, "AA:AA:AA:AA:AA:01"),
            Row::new("garage-pi", -60, "AA:AA:AA:AA:AA:02"),
            Row::new("MY-PI", -70, "AA:AA:AA:AA:AA:03"),
            Row::new("", -70, "AA:AA:AA:AA:AA:04"),
            Row::new("bad\x1bname", -70, "AA:AA:AA:AA:AA:05"),
        ];
        assert_eq!(target(&rows, 0), "AA:AA:AA:AA:AA:01");
        assert_eq!(target(&rows, 1), "garage-pi");
        assert_eq!(target(&rows, 2), "AA:AA:AA:AA:AA:03");
        assert_eq!(target(&rows, 3), "AA:AA:AA:AA:AA:04");
        assert_eq!(target(&rows, 4), "AA:AA:AA:AA:AA:05");
        assert_eq!(note(&rows, 0), None);
        assert_eq!(note(&rows, 2).as_deref(), Some("same name as 1"));
        assert_eq!(note(&rows, 3), None);
    }
}
