//! The channel that last worked for each server (PSM and the client's
//! receive MTU), tried first next time: a channel that stalls costs the
//! stall timeout on every session otherwise. Local to this user; a wrong
//! or stale entry only changes the order of attempts.

use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("blessh").join("channels"))
}

/// The channel that worked last time for `server`: PSM and receive MTU (0
/// when the platform does not choose one).
pub fn get(server: &str) -> Option<(u16, u16)> {
    let text = std::fs::read_to_string(path()?).ok()?;
    text.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        (f.next()? == server).then_some(())?;
        Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
    })
}

/// Remembers the channel that worked for `server`.
pub fn put(server: &str, psm: u16, mtu: u16) {
    let Some(p) = path() else { return };
    if get(server) == Some((psm, mtu)) {
        return;
    }
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| l.split_whitespace().next() != Some(server))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{server} {psm} {mtu}"));
    // A bounded file: the most recent servers.
    let keep = lines.len().saturating_sub(256);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, lines[keep..].join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// The device identifier a server name had last time (macOS identifies
/// devices by CoreBluetooth identifiers).
#[cfg(target_os = "macos")]
pub fn device_of(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(path()?.with_file_name("devices")).ok()?;
    text.lines().find_map(|l| {
        let (n, id) = l.split_once(' ')?;
        blessh_core::name::same(n, name).then(|| id.to_owned())
    })
}

/// Remembers the device identifier of a server name.
#[cfg(target_os = "macos")]
pub fn put_device(name: &str, id: &str) {
    let Some(p) = path().map(|p| p.with_file_name("devices")) else {
        return;
    };
    if device_of(name).as_deref() == Some(id) {
        return;
    }
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| {
            l.split(' ')
                .next()
                .is_some_and(|n| !blessh_core::name::same(n, name))
        })
        .map(str::to_owned)
        .collect();
    lines.push(format!("{name} {id}"));
    let keep = lines.len().saturating_sub(256);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, lines[keep..].join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// `attempts` with the one that worked last time for `server` first.
pub fn order(server: &str, mut attempts: Vec<(u16, u16)>) -> Vec<(u16, u16)> {
    if let Some(last) = get(server) {
        if let Some(i) = attempts.iter().position(|a| *a == last) {
            let a = attempts.remove(i);
            attempts.insert(0, a);
        }
    }
    attempts
}

/// Held while this process opens a channel to a server: the blessh
/// processes of one user open channels to one server one at a time
/// (a CoreBluetooth server refused one of two channel requests that came
/// at once over one link). Released on drop.
pub struct OpenLock(#[allow(dead_code)] Option<std::fs::File>);

impl OpenLock {
    pub async fn take(server: &str) -> Self {
        use std::os::fd::AsRawFd;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let name: String = server
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("blessh-open-{name}.lock")))
        else {
            return Self(None);
        };
        // Opening a channel takes seconds at most; do not wait forever for
        // a process that hangs.
        for _ in 0..300 {
            // SAFETY: flock on an open file.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Self(Some(file));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Self(None)
    }
}
