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
