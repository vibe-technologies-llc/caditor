#![no_main]

use caditor::fuzzing::{parse_stored, preferences, stored_text};
use caditor_file::fuzzing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Some(settings) = fuzzing::settings(bytes) {
        let settled = preferences(settings);
        assert_eq!(
            settled.rewritten, settled.written,
            "preferences changed when saved and loaded again"
        );
        if let Some(stored) = fuzzing::stored_settings(&settled.written) {
            assert_eq!(fuzzing::settings(&stored), Some(settled.written));
        }
    }
    for line in String::from_utf8_lossy(bytes).lines() {
        if let Some(shortcut) = parse_stored(line) {
            assert_eq!(parse_stored(&stored_text(&shortcut)), Some(shortcut));
        }
    }
});
