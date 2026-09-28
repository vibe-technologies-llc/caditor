#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = caditor_file::parse_step(text, "fuzzed.step");
});
