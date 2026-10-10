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
- The port exports no error codes, so `ZstdError` classifies the failure from the negated return
  value against the numbers in `code`, a test pins each; a code it does not know is
  `Unclassified { code }`.
