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

These apply as soon as saving exists:

- Write files atomically: write to a temporary file in the same directory, fsync it, rename it
  over the target, then fsync the directory. Never truncate a user file in place.
- Keep a crash-recovery journal of document changes next to the file, flushed often enough that
  a crash loses at most seconds of work. On startup, offer to restore from it.
- Install a panic hook that flushes the journal before the process ends.
- Version the file format, and keep every format that has shipped readable forever.
- Loading a damaged file recovers everything it can and reports what was lost. It never refuses
  the whole file over one bad feature.
