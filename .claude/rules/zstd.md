---
paths:
  - "crates/caditor-zstd/**"
---

# caditor-zstd

- Safe `compress`, `compress_after`, `decompress` and `decompress_after` over Trifecta Tech
  Foundation's pure-Rust zstd port (`libzstd-rs-sys`). No workspace dependencies; only
  `caditor-file` uses it.

## Deltas

- The `_after` pair takes a raw prefix (the newer version) that makes the frame a delta: the window
  is sized to reach across prefix and data, with long-distance matching. Decoding accepts windows up
  to zstd's maximum, since the output is allocated at the checked size anyway.

## Safety

- The only crate with `unsafe`: its lints set `unsafe_code = "deny"` and each FFI-style call is
  allowed at its own item. Contexts are owned by guards that free them on drop.
- Frames must record their content size. Decompression refuses a frame larger than the caller's
  limit, and one that decodes to a different size than it records.
