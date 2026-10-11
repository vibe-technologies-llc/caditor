# caditor platforms

Linux is the primary platform and Windows the only other one; macOS is not a goal. Packaging,
distribution and checks that need a real machine.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] Windows has been built and tested only in CI (`windows`, `package-windows`)
  and run under wine: no one has used it on a real Windows desktop yet. Check by hand the built-in
  title bar (dragging, Aero Snap, Snap Layouts on the maximize button, resize strips,
  double-click, the corner close, mixed-DPI monitors), the rfd dialogs owned by the window, sign-out flushing the journal, the MSI from
  SmartScreen to uninstall, and a model and its journal on a USB stick (FAT32/exFAT, no POSIX
  rename) and on a network share.
- [medium · medium] The packages ship only caditor's and Inter's licences, though the binary
  carries more: `fonts.rs` keeps egui's default fonts (`epaint_default_fonts`: Hack, Ubuntu Light,
  Noto Emoji and the emoji icon font, under their own licences) in the fallback chain, egui-phosphor
  embeds the Phosphor font, and the statically linked crates are MIT, Apache, BSD, ISC, Zlib and
  Unicode licensed (`deny.toml`'s allow list), most asking for their notice with a binary.
  `packaging/build-release.sh`, `packaging/arch/PKGBUILD`, `packaging/windows/build-release.ps1`
  and `caditor.wxs` install `LICENSE` and `Inter-LICENSE.txt` alone, and the README, the about
  dialog (`about.rs`) and `packaging/INSTALL.md` name only Inter and Phosphor, while
  `docs/RELEASING.md` says the archive holds every licence. Generate a third-party notices file
  from the dependency graph and the font licences, install it beside `LICENSE` in all five
  packages and their checks, and name it in the about dialog.
- [low · medium · blocked by: the project's decision to publish no maintainer identity] The MSI
  and `caditor.exe` are not code-signed, so SmartScreen warns on first run; signing needs a
  certificate tied to an identity.
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] The Linux `.deb`, `.rpm` and AppImage never update themselves: AppImage self-update needs
  update information and a `.zsync` file naming the repository the release is published in, and a
  `.deb` or `.rpm` update needs a package repository to add to the package manager, neither of
  which the project publishes (`docs/RELEASING.md`), so caditor is also not in a software centre.
- [low · medium · blocked by: the project's decision to publish no maintainer identity or repository
  URL] No Flatpak or AUR package: both need a maintainer identity and repository URL in their
  metadata, which the project does not publish (`docs/RELEASING.md`); `packaging/arch/PKGBUILD` only
  builds locally.
