# Reliability

A crash and any loss of the user's work are catastrophic failures, not edge cases.

## No panics in non-test code

The workspace denies `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `todo`,
`unimplemented` and `unreachable`; see the root `Cargo.toml`. `clippy.toml` relaxes them for
tests only.

- Return `Result` with a `thiserror` error in library crates and `anyhow` in the binary. Use
  `.get()` rather than indexing.
- Do not work around the lints with `#[allow]`. If a case seems to need one, restructure the
  code so the invariant is carried by a type.

## Recover instead of exiting

- After startup, a failure in one subsystem degrades that subsystem and leaves the rest
  running. Frame errors are logged and the frame is skipped. A lost surface is recreated inside
  `caditor-render`. Exiting is only for startup failures, when there is nothing to lose yet.
- A failing feature recompute is contained to that feature (see `ux.md`).

## Persistence

`caditor-file` implements these; they hold for anything that writes the user's data:

- Write files atomically: write to a temporary file in the same directory, fsync it, rename it
  over the target, then fsync the directory. Never truncate a user file in place.
- Keep a crash-recovery journal of document changes next to the file, flushed often enough that
  a crash loses at most seconds of work. On startup, offer to restore from it.
- Install a panic hook that flushes the journal before the process ends.
- Version the file format, and keep every format that has shipped readable forever.
- Keep the versions a save replaces inside the file, so earlier states can be restored after a
  restart, and make restoring one an ordinary undoable change. Older versions thin out by age
  (`binary/retention.rs`) so the file does not grow with every save, and thinning never breaks
  a version it keeps.
- Our own formats use zstd for compression, xxh3 for checksums and blake3 for content digests.
  Legacy codecs and checksums (deflate, CRC32) appear only where a foreign format requires them,
  such as the ZIP container of 3MF.
- Loading a damaged file recovers everything it can and reports what was lost. It never refuses
  the whole file over one bad feature.
