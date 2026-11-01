//! CoreBluetooth layer for macOS. The managers' delegates run on a private
//! dispatch queue and report through channels; the caller drives a
//! synchronous state machine on its own thread, and an open L2CAP channel
//! becomes a tokio stream served by a thread of its own.

pub mod central;
pub mod peripheral;
pub mod stream;

use std::{io, ops::Deref, time::Duration};

use objc2::{rc::Retained, Message};
use objc2_core_bluetooth::CBUUID;
use objc2_foundation::{NSError, NSString};

/// A CoreBluetooth object used from several threads.
///
/// CoreBluetooth objects may be called from any thread; their delegate
/// methods run on the dispatch queue given at creation. objc2 marks the
/// classes as neither Send nor Sync because Objective-C makes no such
/// promise in general, hence this wrapper.
pub struct Shared<T: Message>(pub Retained<T>);

// SAFETY: see the type's documentation.
unsafe impl<T: Message> Send for Shared<T> {}
unsafe impl<T: Message> Sync for Shared<T> {}

impl<T: Message> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Message> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub(crate) fn cb_uuid(uuid: &uuid::Uuid) -> Retained<CBUUID> {
    // SAFETY: the string is a well-formed UUID.
    unsafe { CBUUID::UUIDWithString(&NSString::from_str(&uuid.to_string())) }
}

pub(crate) fn uuid_of(uuid: &CBUUID) -> String {
    // SAFETY: no preconditions.
    unsafe { uuid.UUIDString() }.to_string().to_lowercase()
}

/// Whether this program may use Bluetooth (TCC). macOS answers for the
/// responsible process: run from a terminal without respawning, that is
/// the terminal app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allowed,
    Denied,
    Undecided,
}

pub fn permission() -> Permission {
    use objc2_core_bluetooth::CBManagerAuthorization as A;
    // SAFETY: no preconditions.
    match unsafe { objc2_core_bluetooth::CBManager::authorization_class() } {
        A::AllowedAlways => Permission::Allowed,
        A::NotDetermined => Permission::Undecided,
        _ => Permission::Denied,
    }
}

/// Uses Bluetooth once, so that macOS asks for the permission if it has
/// not been decided, and waits up to `timeout` for Bluetooth to be on.
pub fn ask_permission(timeout: Duration) -> io::Result<()> {
    central::Central::new(timeout).map(drop)
}

/// Why Bluetooth did not come up in time: the system's permission prompt
/// is the usual reason for a command-line tool.
pub(crate) fn not_powered_on() -> String {
    // SAFETY: no preconditions.
    let auth = unsafe { objc2_core_bluetooth::CBManager::authorization_class() };
    if auth == objc2_core_bluetooth::CBManagerAuthorization::NotDetermined {
        "waiting for Bluetooth timed out: macOS has not decided whether this program may use \
         Bluetooth; answer its prompt on the Mac's screen (it names blessh), \
         or allow it under System Settings, Privacy & Security, Bluetooth"
            .into()
    } else {
        format!("waiting for Bluetooth timed out (authorization {auth:?})")
    }
}

pub(crate) fn error_text(error: Option<&NSError>) -> String {
    error
        .map(|e| e.localizedDescription().to_string())
        .unwrap_or_else(|| "unknown error".into())
}

/// Whether an error on a channel stream means that the channel or the link
/// ended rather than a fault.
pub fn is_link_end(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::TimedOut
            | io::ErrorKind::UnexpectedEof
    )
}
