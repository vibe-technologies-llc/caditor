# Releasing caditor

## How caditor is distributed

caditor ships as plain release binaries, published as a GitHub release of the version's tag: an
archive for 64-bit Linux, `caditor-<version>-linux-x86_64.tar.zst`, and an installer for 64-bit
Windows, `caditor-<version>-windows-x86_64.msi`, each with a `.sha256` next to it. The archive holds the program, an installer (`install.sh`, into
`~/.local` or any prefix), the menu entry, the icon (the scalable SVG and PNG renders from 16
to 512 px, made by `packaging/render-icons.sh` and committed), the AppStream metainfo, the
`.caditor` MIME type (matched by extension and by the model magic), the documentation and every
licence. `packaging/INSTALL.md` is what users read.

Flatpak and AUR packages were not chosen: both need a maintainer identity and repository URLs
published in their metadata, which the project does not publish. The archive's `share/` tree
follows the freedesktop layout, so a distribution can package it without changes. For the same
reason the desktop ID is plain `caditor` rather than a reverse-DNS name, and the metainfo has no
homepage or developer; `appstreamcli` reports all three (`cid-desktopapp-is-not-rdns`,
`url-homepage-missing`, `developer-info-missing`), and the build accepts exactly those and fails on
any other finding.

`packaging/arch/PKGBUILD` is for building a package on your own Arch machine only: it has no
maintainer, URL or source download, and builds the checkout it sits in
(`cd packaging/arch && makepkg -si`), installing the same files as the archive under `/usr`. Its
`pkgver` must equal the workspace version, which a test in `about.rs` checks.

Release builds drop debug information but keep symbol names (`strip = "debuginfo"`), so the
backtrace a panic writes to the log names the functions involved.

### Windows

The Windows installer is a per-user MSI built with WiX 5 (`packaging/windows/caditor.wxs`): it
installs into `%LOCALAPPDATA%\Programs\caditor` without asking for administrator rights, adds a
Start menu shortcut and opens `.caditor` files, all under the current user's registry, and Apps &
Features uninstalls it. It has no wizard, since there is nothing to choose and the AGPL is not a
licence to accept. WiX 5 rather than a later version: WiX 6 and later come under the Open Source
Maintenance Fee EULA, which a build would have to accept. The MSI is not code-signed (signing
needs an identity the project does not publish), so SmartScreen warns the first time it runs.

The program is built with the MSVC toolchain and links only system DLLs; Direct3D 12, Vulkan and
OpenGL drivers are loaded at run time. It needs Windows 10 version 1809 or later on x86_64. The
executable carries its icon, version information and a manifest (per-monitor DPI awareness, long
paths, common controls 6) built by `crates/caditor/build.rs`.

### Linux

The binary links only glibc and `libgcc_s`; Vulkan, OpenGL, Wayland and X11 libraries are loaded
at run time. It needs the glibc it was built against or newer, so published archives are built
on Ubuntu 22.04 (glibc 2.35) by the release workflow, never on a developer's machine: an
archive built on a rolling distribution runs only on systems as new as it.

## Versions

Versions follow semantic versioning on the workspace version in the root `Cargo.toml`. Before
1.0, the minor number grows with features and the patch number with fixes only. Model files are
not versioned by release: every file format that has shipped stays readable (see
`.claude/rules/reliability.md`).

## Pins

Every toolchain, tool and action CI and the release use is pinned (rustup and cargo-about once
per platform, and WiX), in `.github/versions.env` and
by commit in the workflows. `.github/bump-pins.sh` raises them all to their latest releases with
fresh checksums; commit its diff once CI passes on it.

## Making a release

1. On an up-to-date `master`, run the checks:
   `rust-formatter --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace`.
2. Set the version in `[workspace.package]` in the root `Cargo.toml` if it is not already the
   one being released, then `cargo build` so `Cargo.lock` follows.
3. Preview the archive with `packaging/build-release.sh --snapshot` (it needs `cargo-about`,
   `desktop-file-utils`, `appstream` and `zstd`) and check it with
   `packaging/check-install.sh target/dist/caditor-<version>-snapshot-linux-x86_64.tar.zst`,
   which installs it into a temporary prefix whose name holds a space, `&` and `%`, checks every
   file, the menu entry and the program, uninstalls it, and checks that a failed install leaves
   nothing behind. CI runs both on every push.
   On Windows, `packaging/windows/build-release.ps1 -Snapshot` (it needs `cargo-about` and the
   WiX 5 .NET tool, `dotnet tool install --global wix --version 5.0.2`) builds
   `target/dist/caditor-<version>-snapshot-windows-x86_64.msi`, and
   `packaging/windows/check-install.ps1` installs it, checks the files, the program, the shortcut
   and the file association, installs over it, and uninstalls it. CI runs both on every push.
4. Commit as `Release <version>`, tag it `v<version>` with `git tag -a`, and push `master` and
   the tag.
5. The `Release` workflow (`.github/workflows/release.yml`) first runs the whole CI workflow
   on the tagged commit (tests on lavapipe, clippy, the formatting check, `cargo deny`, the packaging check and the
   fuzzing, through `workflow_call`), and only when it passes builds the archive with
   `packaging/build-release.sh` in an Ubuntu 22.04 container on the toolchain pinned by
   `RUST_TOOLCHAIN` in `.github/versions.env` (which CI reads too), checks it with
   `packaging/check-install.sh`, attests its build provenance through GitHub's Sigstore
   attestations; alongside, it builds, checks and attests the MSI on Windows the same way, and
   then publishes the GitHub release with the archive, the installer, their checksums and
   GitHub's generated notes. GitHub attaches the tagged source, which is the corresponding source the AGPL asks
   for.

`packaging/build-release.sh` refuses to build a release from a dirty tree or from a commit that
is not tagged `v<version>`, lists the release in the metainfo dated by the tagged commit, and
checks the desktop entry, the metainfo and that the binary reports the version.
