//! Session preamble parser: never panics; what parses re-encodes losslessly.
#![no_main]

use blessh_core::session::Preamble;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|line: &str| {
    if let Ok(p) = Preamble::parse(line) {
        let enc = p.encode();
        assert_eq!(Preamble::parse(enc.trim_end_matches('\n')), Ok(p));
    }
});
