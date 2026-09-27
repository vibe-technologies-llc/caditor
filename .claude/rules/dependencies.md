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
- Once a workspace exists, declare shared crates once under `[workspace.dependencies]` and have
  members refer to them with `<crate>.workspace = true`, so each version lives in a single place.
- Commit `Cargo.lock`.
- After editing a `Cargo.toml`, run `rust-formatter` on it, because it formats TOML too.
