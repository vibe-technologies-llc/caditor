# Releasing caditor

## How caditor is distributed

caditor ships as plain release binaries, published as a GitHub release of the version's tag: an
archive for 64-bit Linux, `caditor-<version>-linux-x86_64.tar.zst`, a `.deb`, an `.rpm` and an
`.AppImage` of the same name made from that archive, and an installer for 64-bit
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
`packaging/check-binary.sh PROGRAM` holds the program to this: its needed libraries are glibc and
`libgcc_s` only, `ldd` finds them all, and the highest glibc symbol version is at most 2.35
(`CADITOR_MAX_GLIBC` names another limit, to preview an archive built on a newer system).

`packaging/build-packages.sh ARCHIVE` makes the `.deb`, `.rpm` and `.AppImage` from the archive
itself, not from a second build, so each holds the archive's program, menu entry, icons, metainfo,
MIME type and licences, and all four are one build. It needs `ar` and `objdump` (binutils), `xz`,
`rpmbuild` (the `rpm` package) and `mksquashfs` (`squashfs-tools`) and runs without root. The
packages are reproducible: the same archive gives the same bytes (`SOURCE_DATE_EPOCH` is the
tagged commit's time). `packaging/check-packages.sh [--install] ARCHIVE` checks them: checksums,
metadata, exactly the archive's files in each, the menu entry, and that the program of each runs
and reports the archive's version; `--install` also installs the `.deb` with `apt-get` as root
and removes it again, so it belongs in a disposable container. After installing it expects every
file of the package on disk except those dpkg's own `path-exclude` settings leave out (minimal
Ubuntu images, the CI container among them, drop `/usr/share/doc` but its copyright files). CI runs both on every push.

- The `.deb` is written with `ar` and `tar` (format 2.0, `xz` members, which every dpkg since 2010
  reads) without `dpkg-deb`. Its `Depends` are `libc6` at the highest glibc version the program
  needs (read from the binary, so it follows the build), `libgcc-s1`, `libxkbcommon0`,
  `libvulkan1` and `hicolor-icon-theme`; the libraries the program opens at run time for one
  windowing system or the other (Wayland, X11, EGL) and a file dialog provider (the desktop
  portal or zenity) are `Recommends`, since a Wayland-only or X11-only system needs only half of
  them. The `Maintainer` field is the plain word `caditor`, since the project publishes no
  identity, and there is no `Homepage`. Menu, MIME and icon caches are refreshed by dpkg's file
  triggers, so the package has no maintainer scripts.
- The `.rpm` is built with `rpmbuild` from a spec the script writes around the staged files (a
  gzip payload, which every rpm reads), with no `%changelog`, `Packager`, `URL` or `Vendor`. The
  glibc and `libgcc_s` requirements are the ones `rpmbuild` finds in the program, plus
  `libxkbcommon`, `vulkan-loader` and `hicolor-icon-theme`; the run-time libraries and the
  desktop portal are `Recommends`. Package names are Fedora's and the RHEL family's; other rpm
  distributions (openSUSE names `libvulkan1` and `libxkbcommon0`) install it with
  `--nodeps` or from the archive.
- The `.AppImage` is the pinned type2 runtime (`APPIMAGE_RUNTIME_VERSION` and
  `APPIMAGE_RUNTIME_SHA256` in `.github/versions.env`, raised by `bump-pins.sh`, downloaded once
  into `target/appimage-runtime` and checked against its SHA-256) followed by a zstd
  squashfs of an `AppDir`, joined with `cat` instead of `appimagetool`, which would need FUSE to
  run and downloads its own runtime. The `AppDir` holds `usr/` as the archive's `bin/` and
  `share/`, an `AppRun` that starts `usr/bin/caditor`, the menu entry without `TryExec`
  (which would look for `caditor` on the `PATH`), and the 256 px icon. It bundles no libraries:
  the graphics stack comes from the host like the archive's does, so it runs where the archive
  does. Opening it needs FUSE 2 or 3 on the host; without it `./caditor-*.AppImage
  --appimage-extract-and-run` works. The runtime is MIT licensed and statically links libfuse
  and squashfuse, whose licences are listed in its own repository.
- AppImage self-update is left out: it needs update information (`gh-releases-zsync|<owner>|
  <repository>|...`) naming the repository the release is published in, which the project does not
  publish, and a `.zsync` file for each release. Neither it nor a package repository exists, so
  installs of the `.deb` and `.rpm` do not update themselves either; users download a newer one.

## Versions

Versions follow semantic versioning on the workspace version in the root `Cargo.toml`. Before
1.0, the minor number grows with features and the patch number with fixes only. Model files are
not versioned by release: every file format that has shipped stays readable (see
`.claude/rules/reliability.md`).

## Pins

Every toolchain, tool and action CI and the release use is pinned (rustup once
per platform, WiX and the AppImage runtime), in `.github/versions.env` and
by commit in the workflows. `.github/bump-pins.sh` raises them all to their latest releases with
fresh checksums; commit its diff once CI passes on it.

## Making a release

1. On an up-to-date `master`, run the checks:
   `rust-formatter --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (as CI runs it,
   so the `fuzzing` modules are linted) and `cargo test --workspace --locked`.
2. Set the version in `[workspace.package]` in the root `Cargo.toml` if it is not already the
   one being released, then `cargo build` so `Cargo.lock` follows.
3. Preview the archive with `packaging/build-release.sh --snapshot` (it needs
   `desktop-file-utils`, `appstream` and `zstd`) and check it with
   `packaging/check-install.sh target/dist/caditor-<version>-snapshot-linux-x86_64.tar.zst`,
   which installs it into a temporary prefix whose name holds a space, `&` and `%`, checks every
   file, the menu entry and the program, uninstalls it, and checks that a failed install leaves
   nothing behind and that an upgrade failing midway leaves the earlier install as it was
   (`install.sh` copies every file to a `.caditor-new` name beside its target and renames them
   into place only once every copy succeeded). It also runs `check-binary.sh` on the installed
   program and, when `Xvfb` is installed (always in CI, `CADITOR_REQUIRE_DISPLAY=1`),
   `packaging/check-run.sh`, which starts the program under it on Mesa's software Vulkan driver
   and then on its OpenGL one (`WGPU_BACKEND=gl`) with `CADITOR_STARTUP_CHECK=1` to a first frame
   and a journalled edit, kills it with SIGKILL and starts it again to restore the edit, for an
   untitled document and for a saved model with its journal beside it. CI runs both on every push.
   `packaging/build-packages.sh target/dist/caditor-<version>-snapshot-linux-x86_64.tar.zst` then
   builds the `.deb`, `.rpm` and `.AppImage` beside it (it also needs `rpm`, `squashfs-tools`,
   `xz` and `binutils`), and `packaging/check-packages.sh` on the same archive checks them.
   On Windows, `packaging/windows/build-release.ps1 -Snapshot` (it needs the
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
   `packaging/check-install.sh`, builds the `.deb`, `.rpm` and `.AppImage` from it with
   `packaging/build-packages.sh`, checks them with `packaging/check-packages.sh --install`,
   attests the build provenance of all four through GitHub's Sigstore
   attestations; alongside, it builds, checks and attests the MSI on Windows the same way, and
   then publishes the GitHub release with the archive, the three Linux packages, the installer,
   their checksums and GitHub's generated notes. GitHub attaches the tagged source, which is the
   corresponding source the AGPL asks for.

`packaging/build-release.sh` refuses to build a release from a dirty tree or from a commit that
is not tagged `v<version>`, lists the release in the metainfo dated by the tagged commit, and
checks the desktop entry, the metainfo and that the binary reports the version.
