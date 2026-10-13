#![no_main]

use caditor::fuzzing::mcp_lines;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let lines = bytes.split(|byte| *byte == b'\n').count();
    assert!(mcp_lines(bytes) <= lines);
});
