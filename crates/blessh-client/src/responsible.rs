//! macOS attributes a program's Bluetooth permission (TCC) to its
//! "responsible" process: the terminal app, sshd or a script's launcher
//! rather than blessh. A process started over SSH then has no permission at
//! all. blessh therefore runs itself again as its own responsible process,
//! as terminals do for their shells, so that the permission is granted once
//! to blessh (bundle identifier org.blessh) and holds wherever it runs.

use std::{
    ffi::{CString, OsStr},
    os::unix::ffi::OsStrExt,
    ptr,
    sync::atomic::{AtomicI32, Ordering},
};

/// Set in the child, which then does the work.
const ENV: &str = "BLESSH_RESPONSIBLE";

extern "C" {
    // libSystem since macOS 10.14; private, but what terminals and Chromium
    // use for the same purpose.
    fn responsibility_spawnattrs_setdisclaim(
        attrs: *mut libc::posix_spawnattr_t,
        disclaim: libc::c_int,
    ) -> libc::c_int;
}

static CHILD: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward(sig: libc::c_int) {
    let pid = CHILD.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe.
        unsafe { libc::kill(pid, sig) };
    }
}

/// In the child: exits when the parent is gone (killed, so it could not
/// pass a signal on), instead of holding the link and the locks.
fn watch_parent() {
    // SAFETY: no preconditions.
    let parent = unsafe { libc::getppid() };
    let _ = std::thread::Builder::new()
        .name("blessh-parent".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            // SAFETY: no preconditions.
            if unsafe { libc::getppid() } != parent {
                std::process::exit(1);
            }
        });
}

fn cstring(s: &OsStr) -> Option<CString> {
    CString::new(s.as_bytes()).ok()
}

/// Runs this program again as its own responsible process and returns its
/// exit code, or None when this is that process (or it cannot be started,
/// in which case the caller does the work itself).
pub fn respawn() -> Option<i32> {
    if std::env::var_os(ENV).is_some() {
        watch_parent();
        return None;
    }
    let exe = cstring(std::env::current_exe().ok()?.as_os_str())?;
    let args: Vec<CString> = std::env::args_os()
        .map(|a| cstring(&a))
        .collect::<Option<_>>()?;
    let mut env: Vec<CString> = std::env::vars_os()
        .filter_map(|(k, v)| {
            let mut kv = k.as_bytes().to_vec();
            kv.push(b'=');
            kv.extend(v.as_bytes());
            CString::new(kv).ok()
        })
        .collect();
    env.push(CString::new(format!("{ENV}=1")).ok()?);
    let mut argv: Vec<*mut libc::c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(ptr::null_mut());
    let mut envp: Vec<*mut libc::c_char> = env.iter().map(|e| e.as_ptr().cast_mut()).collect();
    envp.push(ptr::null_mut());

    // SAFETY: the attributes are initialised before use and destroyed after;
    // argv and envp are null-terminated arrays of valid C strings that
    // outlive the call.
    let pid = unsafe {
        let mut attr: libc::posix_spawnattr_t = ptr::null_mut();
        if libc::posix_spawnattr_init(&mut attr) != 0 {
            return None;
        }
        let ok = responsibility_spawnattrs_setdisclaim(&mut attr, 1) == 0;
        let mut pid: libc::pid_t = 0;
        let r = if ok {
            libc::posix_spawn(
                &mut pid,
                exe.as_ptr(),
                ptr::null(),
                &attr,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        } else {
            -1
        };
        libc::posix_spawnattr_destroy(&mut attr);
        if r != 0 {
            return None;
        }
        pid
    };
    CHILD.store(pid, Ordering::SeqCst);
    // ssh ends its ProxyCommand with SIGHUP or SIGTERM; a terminal's ^C
    // reaches the child through the process group anyway.
    // SAFETY: the handler only calls kill.
    unsafe {
        for sig in [libc::SIGHUP, libc::SIGTERM] {
            libc::signal(
                sig,
                forward as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    let mut status = 0;
    loop {
        // SAFETY: waits for our own child.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r == pid {
            break;
        }
        if r < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return Some(1);
        }
    }
    Some(if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    })
}
