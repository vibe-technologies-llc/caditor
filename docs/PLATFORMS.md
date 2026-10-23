# caditor platforms

Linux is the primary platform and Windows the only other one; macOS is not a goal. Packaging,
distribution and checks that need a real machine.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] Windows is used by hand on a real desktop, but a model and its journal on a USB
  stick (FAT32/exFAT, no POSIX rename) and on a network share are not yet checked there.
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
