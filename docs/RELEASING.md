# Releasing caditor

## How caditor is distributed

caditor ships as plain release binaries: one archive per release for 64-bit Linux,
`caditor-<version>-linux-x86_64.tar.zst`, with a `.sha256` next to it, published as a GitHub
release of the version's tag. The archive holds the program, an installer (`install.sh`, into
`~/.local` or any prefix), the menu entry, the icon, the AppStream metainfo, the `.caditor` MIME
type (matched by extension and by the model magic), the documentation and every licence.
`packaging/INSTALL.md` is what users read.

Flatpak and AUR packages were not chosen: both need a maintainer identity and repository URLs
published in their metadata, which the project does not publish. The archive's `share/` tree
follows the freedesktop layout, so a distribution can package it without changes. For the same
reason the desktop ID is plain `caditor` rather than a reverse-DNS name, and the metainfo has no
homepage or developer; `appstreamcli` warns about both and the build accepts those warnings.

The binary links only glibc and `libgcc_s`; Vulkan, OpenGL, Wayland and X11 libraries are loaded
at run time. It needs the glibc it was built against or newer, so published archives are built
on Ubuntu 22.04 (glibc 2.35) by the release workflow, never on a developer's machine: an
archive built on a rolling distribution runs only on systems as new as it.

## Versions

Versions follow semantic versioning on the workspace version in the root `Cargo.toml`. Before
1.0, the minor number grows with features and the patch number with fixes only. Model files are
not versioned by release: every file format that has shipped stays readable (see
`.claude/rules/reliability.md`).

## Changelog

`CHANGELOG.md` lists every release, newest first. A change that users notice adds its line under
`## [Unreleased]` in the same commit, written for users (what they can now do, what behaves
differently, what they must do), not as a commit message.

## Making a release

1. On an up-to-date `master`, run the checks:
   `rust-formatter --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace`.
2. Set the version in `[workspace.package]` in the root `Cargo.toml` if it is not already the
   one being released, then `cargo build` so `Cargo.lock` follows.
3. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [<version>] - <YYYY-MM-DD>` and open a new,
   empty `## [Unreleased]` above it.
4. Preview the archive with `packaging/build-release.sh --snapshot` (it needs `cargo-about`,
   `desktop-file-utils`, `appstream` and `zstd`), extract it, and try `./install.sh --prefix`
   into a temporary directory.
5. Commit as `Release <version>`, tag it `v<version>` with `git tag -a`, and push `master` and
   the tag.
6. The `Release` workflow (`.github/workflows/release.yml`) runs the tests and clippy in an
   Ubuntu 22.04 container, builds the archive with `packaging/build-release.sh`, and publishes
   the GitHub release with the archive, its checksum and the changelog section as notes. GitHub
   attaches the tagged source, which is the corresponding source the AGPL asks for.

`packaging/build-release.sh` refuses to build a release from a dirty tree, from a commit that
is not tagged `v<version>`, or without a dated changelog section for the version, and checks the
desktop entry, the metainfo and that the binary reports the version.
