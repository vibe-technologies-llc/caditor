# Reliability

A crash or any loss of the user's work is a catastrophic failure, not an edge case.

## No panics in non-test code

- The workspace denies `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `todo`,
  `unimplemented` and `unreachable` (root `Cargo.toml`); `clippy.toml` relaxes them for tests.
- Return `Result`: `thiserror` errors in library crates, `anyhow` in the binary. Use `.get()`, not
  indexing.
- Never `#[allow]` around the lints; restructure so a type carries the invariant.

## Recover instead of exiting

- After startup, a failing subsystem degrades alone and the rest keeps running: frame errors are
  logged and the frame skipped, a lost surface is recreated inside `caditor-render`, a failing
  feature is contained to that feature. Exiting is only for startup failures.

## Persistence

`caditor-file` implements these; they hold for anything writing the user's data:

- Write atomically: a temporary in the same directory, fsync, rename over the target, fsync the
  directory. Never truncate a user file in place.
- Keep a recovery journal of changes next to the file, flushed so a crash loses at most seconds,
  and offer to restore from it at startup. A panic hook flushes it before the process ends.
- Version the format and keep every shipped format readable forever.
- Keep the versions a save replaces inside the file; restoring one is an ordinary undoable change.
  Old versions thin out by age (`binary/retention.rs`) without breaking any version kept.
- Our formats use zstd for compression, xxh3 for checksums and blake3 for content digests; deflate
  and CRC32 appear only where a foreign format needs them (3MF's ZIP).
- Loading a damaged file recovers everything it can and reports what was lost, never refusing the
  whole file over one bad feature.
