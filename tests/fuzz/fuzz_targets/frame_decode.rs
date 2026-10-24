//! Any packet: decoding never panics, and whatever decodes re-encodes to a
//! packet that decodes to the same frame.
#![no_main]

use ble_term::protocol::Frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|pkt: &[u8]| {
    if let Ok((seq, frame)) = Frame::decode(pkt) {
        let again = frame.encode(seq);
        assert_eq!(Frame::decode(&again), Ok((seq, frame)));
    }
});
