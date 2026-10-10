---
paths:
  - "crates/caditor-zstd/**"
---

# caditor-zstd

- Safe wrapper over Trifecta Tech Foundation's Rust zstd port (`libzstd-rs-sys`, pre-release);
  only `caditor-file` uses it.
- The port is pure Rust except `huf_decompress_amd64.S`, which its `build.rs` assembles with `cc`
  on x86_64 Unix, so building needs a C compiler (`gcc` in the PKGBUILD's `makedepends`); no C
  library is linked. No other Rust zstd covers prefix deltas, so the exception stands.
- The `_after` functions take a raw prefix (the newer version) that makes the frame a delta: the
  compression window reaches across prefix and data, with long-distance matching. Decoding accepts
  any window up to zstd's maximum, since the output is allocated at the checked size anyway.
- One of the three crates with `unsafe` (with `caditor-windows` and `caditor-wayland`;
  `unsafe_code = "deny"`, allowed per call site); contexts are owned by guards that free them on drop.
- The decoder reads untrusted data: frames must record their content size, and decompression
  refuses one larger than the caller's limit or decoding to a different size than recorded.
- The port works out the history size as the output's end minus the prefix's address before it
  checks for an empty output, which overflows when a frame holding sequences starts on an empty
  output below the prefix. With overflow checks on (tests, fuzzing, `cargo run`) that panics inside
  `extern "C"` functions, which aborts the process, so `catch_unwind` could not contain it; a panic
  inside the port must be kept from happening at this boundary. Two guards rule it out:
  - Decompression takes exactly one frame: `ZSTD_findFrameCompressedSize` must span the whole
    input, or it is `NotOneFrame`, before anything is decoded. Concatenated frames (one filling the
    output, another its spare byte, a third holding sequences) would otherwise start on an empty
    output, and a skippable frame beside a real one is refused with them; everything we write is
    one frame from `ZSTD_compress2`.
  - The output has `SPARE_ROOM` past the recorded size, so the one frame never starts on an empty
    output even when it records no content but holds sequences.

  The fuzz input that found it is `src/crashes/history-past-output.bin`, kept by a test; the
  three-frame test needs a prefix large enough to be mapped above the output.
- The port exports no error codes, so `ZstdError` classifies the failure from the negated return
  value against the numbers in `code`, a test pins each; a code it does not know is
  `Unclassified { code }`.
