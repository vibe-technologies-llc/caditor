#![no_main]

use caditor_file::fuzzing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = fuzzing::recover(bytes);
});
