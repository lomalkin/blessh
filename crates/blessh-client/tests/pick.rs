//! `blessh` without a server, on a pseudo-terminal: a fake scanner lists
//! servers (BLESSH_SCANNER) and a fake ssh on PATH prints the arguments it
//! got and the terminal's mode, so the list, the keys, the ssh command
//! line, the terminal's restoration and the scanner's end can be checked
//! without Bluetooth.

use std::{
    fs,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const SCANNER: &str = r#"#!/bin/sh
if [ -n "$FAKE_FAIL" ]; then
    echo "blessh: adapter hci0 is powered off" >&2
    exit 3
fi
echo $$ > "$FAKE_DIR/scanner.pid"
printf 'my-pi\t-52\tAA:AA:AA:AA:AA:01\n'
sleep 0.2
printf 'garage-pi\t-67\tAA:AA:AA:AA:AA:02\n'
printf 'MY-PI\t-74\tAA:AA:AA:AA:AA:03\n'
exec sleep 600
"#;

const SSH: &str = r#"#!/bin/sh
for a in "$@"; do printf 'ARG<%s>\n' "$a"; done
stty -a | tr ' ' '\n' | grep -E '^-?(icanon|echo)$' | sed 's/^/TTY</; s/$/>/'
"#;

struct Session {
    dir: PathBuf,
    child: Child,
    master: OwnedFd,
    slave: OwnedFd,
    output: String,
}

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    Command::new("chmod").arg("755").arg(path).status().unwrap();
}

impl Session {
    fn start(name: &str, args: &[&str], env: &[(&str, &str)], typed_before: &[u8]) -> Session {
        let dir = std::env::temp_dir().join(format!("blessh-pick-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        script(&dir.join("scanner"), SCANNER);
        script(&dir.join("bin/ssh"), SSH);
        let (mut m, mut s) = (0, 0);
        // SAFETY: openpty writes two descriptors.
        let r = unsafe {
            libc::openpty(
                &mut m,
                &mut s,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(r, 0, "openpty");
        // SAFETY: fresh descriptors owned here.
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
        if !typed_before.is_empty() {
            write_fd(&master, typed_before);
        }
        let path = format!(
            "{}:{}",
            dir.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let io = || Stdio::from(slave.try_clone().unwrap());
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_blessh"));
        cmd.args(args)
            .env("PATH", path)
            .env("BLESSH_SCANNER", dir.join("scanner"))
            .env("FAKE_DIR", &dir)
            .env("NO_COLOR", "1")
            .env("LANG", "C")
            .env_remove("LC_ALL")
            .env_remove("LC_CTYPE")
            .stdin(io())
            .stdout(io())
            .stderr(io());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().unwrap();
        Session {
            dir,
            child,
            master,
            slave,
            output: String::new(),
        }
    }

    /// Reads what blessh wrote for up to `ms`.
    fn read_for(&mut self, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let mut fd = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            if unsafe { libc::poll(&mut fd, 1, left.as_millis() as i32) } <= 0 {
                break;
            }
            let mut buf = [0u8; 4096];
            // SAFETY: reads into a buffer of its length.
            let n = unsafe { libc::read(fd.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            self.output
                .push_str(&String::from_utf8_lossy(&buf[..n as usize]));
        }
    }

    fn expect(&mut self, text: &str) {
        let end = Instant::now() + Duration::from_secs(10);
        while !self.output.contains(text) {
            assert!(
                Instant::now() < end,
                "no {text:?} in the output:\n{}",
                self.output
            );
            self.read_for(100);
        }
    }

    fn press(&mut self, keys: &[u8]) {
        write_fd(&self.master, keys);
    }

    /// Waits for blessh to exit and returns its code, with the output read.
    fn finish(&mut self) -> i32 {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            self.read_for(50);
            if let Some(status) = self.child.try_wait().unwrap() {
                self.read_for(100);
                return status.code().unwrap_or(-1);
            }
            assert!(
                Instant::now() < end,
                "blessh did not exit:\n{}",
                self.output
            );
        }
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    /// Whether the fake scanner is gone, after a moment to go.
    fn scanner_gone(&self) -> bool {
        let pid: i32 = fs::read_to_string(self.dir.join("scanner.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            // SAFETY: signal 0 only checks the process.
            if unsafe { libc::kill(pid, 0) } != 0 {
                return true;
            }
            if Instant::now() > end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Whether the terminal is back in canonical mode with echo.
    fn terminal_restored(&self) -> bool {
        let mut t = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tcgetattr fills the struct on success.
        assert_eq!(
            unsafe { libc::tcgetattr(self.slave.as_raw_fd(), t.as_mut_ptr()) },
            0
        );
        // SAFETY: initialised above.
        let t = unsafe { t.assume_init() };
        t.c_lflag & libc::ICANON != 0 && t.c_lflag & libc::ECHO != 0
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn write_fd(fd: &OwnedFd, bytes: &[u8]) {
    // SAFETY: writes from a buffer of its length.
    let n = unsafe { libc::write(fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
    assert_eq!(n, bytes.len() as isize);
}

#[test]
fn connects_by_name_to_the_device_listed() {
    let mut s = Session::start("name", &[], &[], b"");
    s.expect("Scanning for blessh servers; 1-9 a-z connect; 0 q Esc quit");
    s.expect(" 1  my-pi ");
    s.expect(" 2  garage-pi ");
    s.press(b"2");
    assert_eq!(s.finish(), 0, "{}", s.output);
    let out = &s.output;
    assert!(out.contains("-> garage-pi"), "{out}");
    assert!(out.contains("ARG<garage-pi>"), "{out}");
    assert!(
        out.contains("proxy --device 'AA:AA:AA:AA:AA:02' 'garage-pi'>"),
        "{out}"
    );
    assert!(out.contains("ARG<HostKeyAlias=blessh-garage-pi>"), "{out}");
    assert!(!out.contains("User="), "{out}");
    assert!(
        out.contains("TTY<icanon>") && out.contains("TTY<echo>"),
        "{out}"
    );
    assert!(s.scanner_gone());
}

#[test]
fn a_duplicate_name_is_reached_by_address() {
    let mut s = Session::start("dup", &[], &[], b"");
    s.expect(" 3  MY-PI ");
    s.expect("same name as 1");
    s.press(b"3");
    assert_eq!(s.finish(), 0, "{}", s.output);
    let out = &s.output;
    assert!(out.contains("ARG<AA:AA:AA:AA:AA:03>"), "{out}");
    assert!(out.contains("proxy 'AA:AA:AA:AA:AA:03'>"), "{out}");
    assert!(out.contains("HostKeyAlias=blessh-aaaaaaaaaa03"), "{out}");
}

#[test]
fn keeps_the_user_options_and_command() {
    let mut s = Session::start("args", &["-p", "22", "alice@", "uname", "-a"], &[], b"");
    s.expect(" 2  garage-pi ");
    s.press(b"2");
    assert_eq!(s.finish(), 0, "{}", s.output);
    let out = &s.output;
    assert!(
        out.contains("ARG<-p>\r\nARG<22>\r\n")
            && out.contains("ARG<alice@garage-pi>\r\nARG<uname>\r\nARG<-a>"),
        "{out}"
    );
}

#[test]
fn quit_keys() {
    for (keys, code) in [
        (&b"q"[..], 0),
        (b"0", 0),
        (b"\x1b", 0),
        (b"\x03", 130),
        (b"\x04", 0),
    ] {
        let mut s = Session::start("quit", &[], &[], b"");
        s.expect(" 1  my-pi ");
        s.press(keys);
        assert_eq!(s.finish(), code, "{keys:?}: {}", s.output);
        assert!(!s.output.contains("ARG<"), "{}", s.output);
        assert!(s.terminal_restored(), "{keys:?}");
        assert!(s.scanner_gone(), "{keys:?}");
    }
}

#[test]
fn ignores_arrows_other_keys_and_keys_typed_before() {
    let mut s = Session::start("ignore", &[], &[], b"1");
    s.expect(" 3  MY-PI ");
    s.press(b"\x1b[A");
    s.press(b"9z\r ");
    s.read_for(500);
    assert!(s.running(), "{}", s.output);
    s.press(b"q");
    assert_eq!(s.finish(), 0, "{}", s.output);
    assert!(!s.output.contains("ARG<"), "{}", s.output);
}

#[test]
fn a_failed_scan_ends_the_list() {
    let mut s = Session::start("fail", &[], &[("FAKE_FAIL", "1")], b"");
    assert_eq!(s.finish(), 3, "{}", s.output);
    assert!(
        s.output.contains("adapter hci0 is powered off"),
        "{}",
        s.output
    );
    assert!(s.terminal_restored());
}

#[test]
fn a_list_needs_a_terminal() {
    let out = Command::new(env!("CARGO_BIN_EXE_blessh"))
        .args(["-l", "alice"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("needs a terminal"));
}
