//! Logger setup: plain `<level>` prefixes under systemd, timestamps otherwise.

use std::io::Write;

pub fn init(default_level: &str) {
    let mut b =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level));
    if std::env::var_os("JOURNAL_STREAM").is_some() {
        b.format(|f, r| {
            let prio = match r.level() {
                log::Level::Error => 3,
                log::Level::Warn => 4,
                log::Level::Info => 6,
                log::Level::Debug | log::Level::Trace => 7,
            };
            writeln!(f, "<{prio}>{}", r.args())
        });
    }
    b.init();
}
