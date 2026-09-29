#![no_main]

use caditor_file::fuzzing;
use libfuzzer_sys::fuzz_target;

const VERSIONS_VISITED: usize = 16;

fuzz_target!(|bytes: &[u8]| {
    let sealed = fuzzing::reseal(bytes);
    let _ = caditor_file::decode(&sealed);
    let history = fuzzing::history(&sealed);
    for version in history.versions.iter().take(VERSIONS_VISITED) {
        let _ = fuzzing::load_version(&sealed, version.index);
    }
});
