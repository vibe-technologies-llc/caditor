---
paths:
  - "crates/caditor-windows/**"
  - "crates/caditor-file/src/os/**"
  - "crates/caditor/src/portal/windows.rs"
  - "crates/caditor/build.rs"
  - "crates/caditor/src/defender.rs"
  - "packaging/windows/**"
  - ".github/install-windows-packaging.sh"
---

# Windows

Linux is the primary platform and Windows (10 1809 or later, x86_64) the only other one; macOS is
not a goal. Platform code is a pair of `cfg(unix)` and `cfg(windows)` items behind one interface,
so call sites stay platform-free.

## `caditor-windows`, the Win32 boundary

- The second crate allowed `unsafe` (after `caditor-zstd`): `unsafe_code = "deny"` with
  `#[allow(unsafe_code)]` on each item, and the workspace clippy lints listed in its manifest
  (`conventions_tests.rs` checks both). Its root is `#![cfg(windows)]`, so it is empty elsewhere
  and only Windows CI and `cargo xwin` lint it. It uses `windows-sys` with only the `Win32_*`
  features it calls, and offers only safe functions:
  - `replace_file`: `ReplaceFileW`, so a save keeps the replaced file's ACLs, attributes,
    alternate streams and creation time. `move_file_durably`: `MoveFileExW` with
    `REPLACE_EXISTING | WRITE_THROUGH`, for a target that does not exist yet. Both take verbatim
    (`\\?\`) paths so long names work.
  - `FileId`: volume serial and 128-bit file ID (`GetFileInformationByHandleEx`, `FileIdInfo`),
    the Windows counterpart of `(dev, ino)`; `of_path` opens without access and without following
    a reparse point.
  - `process_running`: `OpenProcess` plus a zero wait; access denied counts as running.
  - `machine_guid` and `boot_id` read `MachineGuid` and the `PrefetchParameters\BootId` counter
    from the registry for temporary-file tags; either may be missing.
  - `on_console_close` (`SetConsoleCtrlHandler`) and `on_session_end` (the main window's procedure
    replaced with one that calls back on `WM_ENDSESSION` and chains to winit's): each keeps one
    callback in a static. User32 subclassing, not comctl32's `SetWindowSubclass`, so binaries
    without the Common Controls v6 manifest (tests) still load.
  - `attach_parent_console`: a release build is a GUI-subsystem program, so `--version`, `--help`
    and `--export` print only after attaching to the console that started it.
  - `DialogParent`: an HWND as `HasWindowHandle` and `HasDisplayHandle`, for rfd.
  - `show_error`: `MessageBoxW`, for a failed start.

## Files (`caditor-file/src/os/`)

- State in `%LOCALAPPDATA%\caditor` (recovery, logs, recent files), preferences in
  `%APPDATA%\caditor`; XDG only on Unix.
- Saving renames over an existing target with `ReplaceFileW`; if that fails and the temporary is
  still there it falls back to `std::fs::rename` (POSIX semantics, so an open target is replaced).
  There is no directory fsync, no group or xattr copy, and no `copy_file_range`: history versions
  are written from memory. Writable means the read-only attribute is clear.
- Orphaned temporaries use the same tags; the owner process is checked with `process_running`.
- Journal temporaries are created hidden, so the journal is too (a leading dot hides nothing on
  Windows). A journal does not take the model's read-only attribute, since Windows refuses to
  rename over or delete read-only files.
- Locks are mandatory (`LockFileEx`): only the holder reads a locked journal, through its own
  handle. Journals are renamed over while their owner holds them open and locked, which needs NTFS
  POSIX rename semantics; on a volume without them the adjacent journal fails and the next
  candidate, the recovery directory, takes it.
- Paths stored as bytes (journal header, recovery markers, recent files) are WTF-8 from
  `as_encoded_bytes`, decoded back with a safe WTF-8 decoder, so unpaired surrogates survive.

## App

- File dialogs are rfd's (`portal/windows.rs`), owned by the main window once it exists
  (`portal::own_dialogs`). rfd's `common-controls-v6` feature stays off: it imports
  `TaskDialogIndirect`, which unmanifested test binaries cannot load.
- `crash.rs` flushes the journal and ends the log on a console close and when Windows ends the
  session; the periodic journal flush bounds loss otherwise.
- After the first save on Windows (`Files::saved_folder`), `defender.rs` shows once a card over
  the view, like a tip and taking a tip's place, saying that Microsoft Defender's real-time scanning
  slows the atomic saves, the journal and the kept versions, how to exclude the models' folder in
  Windows Security and what that trades away; Read in the guide opens the `windows-defender` page.
  Either choice records `onboarding.defender_reminded`. Preferences › General › Saving on Windows
  (Windows only, `defender::ON_WINDOWS`) shows it again (`PreferencesCommand::ShowDefenderReminder`).
  caditor never reads or changes Defender's settings.
- Windows prefers Direct3D 12, then Vulkan, then OpenGL (`caditor-render` `BACKEND_ORDER`).
- The window gets a class name, a drop shadow while undecorated and a taskbar icon. The built-in
  title bar drags with `StartDrag`, so Aero Snap works; Snap Layouts on the maximize button do not.
- `build.rs` builds `caditor.ico` from the committed PNG renders (PNG entries in an ICO) and embeds
  it with `packaging/windows/caditor.exe.manifest` (PerMonitorV2, long paths, Common Controls v6)
  and version information through `winresource`, using `rc.exe`, or `llvm-rc` when
  cross-compiling.

## Installer (`packaging/windows/`)

- `caditor.wxs` is WiX 5 (MS-RL; later WiX versions carry the OSMF EULA). A per-user MSI with no
  wizard: `%LOCALAPPDATA%\Programs\caditor` with `caditor.exe`, `README.md` and `licenses\` (the
  licence and Inter's), a Start menu shortcut
  and the `.caditor` association (`caditor.model`), all under HKCU. Each component's key path is an
  HKCU value, as per-user components need; WiX cannot derive a GUID for a component holding
  several files that way, so `Program` and `Licences` have fixed ones. Those GUIDs and the
  `UpgradeCode` never change; same-version upgrades are allowed so a snapshot installs over
  itself.
- `build-release.ps1 [-Snapshot]` mirrors `build-release.sh` and writes
  `target/dist/caditor-<label>-windows-x86_64.msi` with its `.sha256`; `check-install.ps1` installs
  it quietly, checks the files, `--version`, shortcut and association, installs over it, then
  uninstalls and checks nothing is left.
