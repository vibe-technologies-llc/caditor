---
paths:
  - "**/Cargo.toml"
  - "Cargo.lock"
---

# Dependencies

- Write every dependency with a full three-part version, `x.y.z`, never `x` or `x.y`. This
  applies to normal, dev, build and workspace dependencies alike.
- Use the latest release when adding or bumping a crate. Look it up with `cargo info <crate>` or
  `cargo search <crate> --limit 1`, not from memory.
- Declare every dependency once, under `[workspace.dependencies]` in the root `Cargo.toml`.
  Members refer to it with `<crate>.workspace = true`, adding only the features they need, so
  each version lives in a single place.
- Internal crates are `path`-only entries in `[workspace.dependencies]` with no version, since
  they are `publish = false`.
- Commit `Cargo.lock`.
- After editing a `Cargo.toml`, run `rust-formatter` on it, because it formats TOML too.
