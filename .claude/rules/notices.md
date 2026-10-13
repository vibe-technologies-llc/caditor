---
paths:
  - "crates/caditor-notices/**"
---

# Third-party notices

- `caditor-notices` writes `THIRD-PARTY-NOTICES.txt`, which every package installs beside `LICENSE`
  (`share/licenses/caditor/` in the archive, `.deb`, `.rpm`, AppImage and Arch package, `licenses\`
  in the MSI). The release scripts run it
  (`cargo run --release --locked -p caditor-notices -- [--target TRIPLE] OUTPUT`) rather than the
  repository committing its output, so the file always describes the dependency graph that was
  built. `about.rs` names the file, and a test there holds every packaging script and check to it.
- The tool is a workspace crate, not `cargo about`, so it needs nothing installed and runs the same
  on the Linux release container and the Windows runner. It reads `cargo metadata --locked
  --filter-platform TRIPLE` and takes the normal (linked) dependencies reachable from `caditor`:
  no dev or build dependencies, no proc-macro crates and nothing reached only through them, and
  none of the other platform's crates. The Linux archive and the MSI therefore list different
  crates.
- A crate's texts are the files at the top of its package whose names start with LICENSE, LICENCE,
  COPYING, UNLICENSE, NOTICE or COPYRIGHT. A crate with none gets the standard text of each of MIT,
  Apache-2.0, BSL-1.0 and BSD-3-Clause its manifest's licence expression names (`licenses/`), MIT
  and BSD-3-Clause with its authors, addresses removed, as the copyright holders (`alloc-stdlib`,
  under `jxl-oxide`'s Brotli decoder, ships no file); one naming none of the three fails the
  build with `NoLicenseText` and gets its text added to `licenses/`, never a silent omission. The
  Rust standard library, which is linked in as well, is listed the same way.
- The fonts are a table in `components.rs` of the licence files they ship with: Inter and Phosphor
  from `crates/caditor/assets/fonts/` (`Inter-LICENSE.txt`, `Phosphor-LICENSE.txt`, both
  committed), and the Hack, Ubuntu Light, Noto Emoji and emoji-icon-font texts from the
  `epaint_default_fonts` package that egui's default fonts come from, so they follow the version
  built. A font added to the program or dropped from it changes that table in the same commit.
- Identical texts are written once with every component carrying it listed beneath, in a stable
  order (assets, then crates by name and version), and line endings and trailing blanks are
  normalised first, so the same graph gives the same bytes on both platforms.
