#![no_main]

use caditor_file::MAX_ENVIRONMENT_PIXELS;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(pixels) = caditor_file::fuzzing::decode_environment(bytes) {
        let texels = u64::from(pixels.width) * u64::from(pixels.height);

        assert!(texels <= MAX_ENVIRONMENT_PIXELS);
        assert_eq!(pixels.rgb.len() as u64, texels * 3);
    }
});
