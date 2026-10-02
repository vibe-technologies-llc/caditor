#![no_main]

use caditor_file::fuzzing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let recent = fuzzing::recent_files(bytes);
    if let Some(stored) = fuzzing::stored_recent_files(&recent) {
        assert_eq!(fuzzing::recent_files(&stored), recent);
    }
});
