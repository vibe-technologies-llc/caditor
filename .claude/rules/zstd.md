---
paths:
  - "crates/caditor-zstd/**"
---

# caditor-zstd

- Safe wrapper over Trifecta Tech Foundation's Rust zstd port (`libzstd-rs-sys`, pre-release); only
  `caditor-file` uses it.
- The port is pure Rust except `huf_decompress_amd64.S`, which its `build.rs` assembles with `cc` on
  x86_64 Unix: building needs a C compiler (`gcc` in the PKGBUILD's `makedepends`), no C library is
  linked. No other Rust zstd covers prefix deltas, so the exception stands.
- The `_after` functions take a raw prefix (the newer version) that makes the frame a delta: the
  window reaches across prefix and data, with long-distance matching. Decoding accepts any window up
  to zstd's maximum, since the output is allocated at the checked size anyway.
- One of the three crates with `unsafe` (with `caditor-windows` and `caditor-wayland`;
  `unsafe_code = "deny"`, expected per call site); guards own the contexts and free them on drop.
- The decoder reads untrusted data: frames must record their content size, and decompression refuses
  one larger than the caller's limit or decoding to a different size than recorded.
- The port computes the history size (output end minus prefix address) before checking for an empty
  output, which overflows when a frame holding sequences starts on an empty output below the prefix.
  With overflow checks on (tests, fuzzing, `cargo run`) that panics inside `extern "C"` code, which
  aborts, so `catch_unwind` cannot contain it; it must never happen at this boundary. Two guards:
  - Decompression takes exactly one frame: `ZSTD_findFrameCompressedSize` must span the whole input,
    else `NotOneFrame`, before decoding. Concatenated frames (one filling the output, another its
    spare byte, a third with sequences) would start on an empty output; a skippable frame beside a
    real one is refused too. Everything we write is one frame from `ZSTD_compress2`.
  - The output has `SPARE_ROOM` past the recorded size, so the one frame never starts on an empty
    output even when it records no content but holds sequences.

  The fuzz input that found it, `src/crashes/history-past-output.bin`, is kept by a test; the
  three-frame test needs a prefix large enough to be mapped above the output.
- The port exports no error codes: `ZstdError` classifies the failure from the negated return value
  against the numbers in `code` (a test pins each); an unknown one is `Unclassified { code }`.
