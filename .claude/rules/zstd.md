---
paths:
  - "crates/caditor-zstd/**"
---

# caditor-zstd

- Safe `compress`, `compress_after`, `decompress` and `decompress_after` over Trifecta Tech
  Foundation's Rust zstd port (`libzstd-rs-sys`, a pre-release). No workspace dependencies; only
  `caditor-file` uses it.
- The port is pure Rust except for one file: on x86_64 Unix its `build.rs` assembles
  `huf_decompress_amd64.S` with `cc` for the Huffman decoder, so building needs a C compiler (`gcc`
  in the PKGBUILD's `makedepends`) and no C library is linked. No other Rust zstd covers prefix
  deltas, so the exception stands. The decoder reads untrusted data, so every frame is checked as
  described under Safety.

## Deltas

- The `_after` pair takes a raw prefix (the newer version) that makes the frame a delta: the window
  is sized to reach across prefix and data, with long-distance matching. Decoding accepts windows up
  to zstd's maximum, since the output is allocated at the checked size anyway.

## Safety

- The only crate with `unsafe`: its lints set `unsafe_code = "deny"` and each FFI-style call is
  allowed at its own item. Contexts are owned by guards that free them on drop.
- Frames must record their content size. Decompression refuses a frame larger than the caller's
  limit, and one that decodes to a different size than it records.
