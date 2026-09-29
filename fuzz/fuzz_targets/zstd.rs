#![no_main]

use caditor_zstd::Level;
use libfuzzer_sys::fuzz_target;

const LIMIT: usize = 1 << 20;

fuzz_target!(|bytes: &[u8]| {
    let Some((&split, rest)) = bytes.split_first() else {
        return;
    };
    let (prefix, data) = rest.split_at(usize::from(split).min(rest.len()));
    let _ = caditor_zstd::decompress(data, LIMIT);
    let _ = caditor_zstd::decompress_after(data, prefix, LIMIT);
    if let Ok(frame) = caditor_zstd::compress_after(data, prefix, Level::FAST) {
        let restored = caditor_zstd::decompress_after(&frame, prefix, data.len());
        assert_eq!(restored.as_deref(), Ok(data));
    }
});
