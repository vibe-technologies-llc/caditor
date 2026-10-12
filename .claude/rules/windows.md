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

- Allowed `unsafe` like `caditor-zstd`: `unsafe_code = "deny"` with `#[expect(unsafe_code, reason)]`
  per item (`conventions_tests.rs` checks it). Root is `#![cfg(windows)]`, so only Windows CI and
  `cargo xwin` lint it. `windows-sys` with only the `Win32_*` features used; only safe functions are
  exported:
  - `replace_file` (`ReplaceFileW`, keeps the replaced file's ACLs, attributes, streams, creation
    time) and `move_file_durably` (`MoveFileExW`, `REPLACE_EXISTING | WRITE_THROUGH`, for a target
    that does not exist yet). Both take verbatim `\\?\` paths.
  - `FileId`: volume serial plus 128-bit file ID, the counterpart of `(dev, ino)`; `of_path` opens
    without access and without following a reparse point.
  - `final_path`: `GetFinalPathNameByHandleW` with the `\\?\` and `\\?\UNC\` prefix removed; fails
    for a missing path.
  - `cluster_size` (`FSCTL_GET_INTEGRITY_INFORMATION`, answers only on ReFS so it is the block
    cloning probe) and `duplicate_extents` (`FSCTL_DUPLICATE_EXTENTS_TO_FILE`, one request per call).
  - `process_running`: `OpenProcess` plus a zero wait; access denied counts as running.
  - `machine_guid` and `boot_id`: registry `MachineGuid` and `PrefetchParameters\BootId`, for
    temporary-file tags; either may be missing.
  - `on_console_close` (`SetConsoleCtrlHandler`) and `on_session_end` (main window procedure
    replaced to call back on `WM_ENDSESSION`, chaining to winit's): one callback each, in a static.
    User32 subclassing, not comctl32's `SetWindowSubclass`, so binaries without the Common Controls
    v6 manifest (tests) still load.
  - `on_maximize_button` and `set_maximize_button`: a second subclass procedure with its own statics,
    giving Snap Layouts on caditor's own maximize button. The app reports the button's rectangle
    (`ButtonRect`, physical client pixels) whenever it changes, `None` with no such button (full
    screen, system title bar). On the button the procedure answers `WM_NCHITTEST` with
    `HTMAXBUTTON` and handles `WM_NCMOUSEMOVE` (arming `TrackMouseEvent` with
    `TME_NONCLIENT | TME_LEAVE`), `WM_NCMOUSELEAVE`, `WM_NCLBUTTONDOWN`, `WM_NCLBUTTONDBLCLK` and
    `WM_NCLBUTTONUP` without chaining, so `DefWindowProc` never runs its caption-button loop. It
    reports a `ButtonState` (idle, hovered, pressed) to a callback, and a release after a press on
    the button maximises or restores with `ShowWindow`. Elsewhere messages chain on, so winit keeps
    resize and move and the corner close stays an egui widget.
  - `battery_flag`: `GetSystemPowerStatus`'s `BatteryFlag`, read by `caditor-render` to tell a
    desktop (128) from a laptop for the Auto graphics adapter.
  - `attach_parent_console`: a release build is GUI-subsystem, so `--version`, `--help` and
    `--export` print only after attaching to the starting console.
  - `DialogParent`: an HWND as `HasWindowHandle` and `HasDisplayHandle`, for rfd.
  - `show_error`: `MessageBoxW`, for a failed start.

## Files (`caditor-file/src/os/`)

- State in `%LOCALAPPDATA%\caditor` (recovery, logs, recent files), preferences in
  `%APPDATA%\caditor`; XDG only on Unix.
- Saving renames over an existing target with `ReplaceFileW`; if that fails and the temporary is
  still there it falls back to `std::fs::rename`. No directory fsync, no group or xattr copy.
  Writable means the read-only attribute is clear.
- `os::clone_range` is ReFS block cloning: it clones only the whole clusters of a range whose source
  and target offsets are both cluster aligned (`os/cloning.rs` holds the arithmetic, tested
  everywhere), in requests of at most 1 GiB, first growing the target with `set_len` to whole
  clusters (the save truncates it at the end). Any error (NTFS, FAT, network shares, another
  volume, sparse or integrity mismatch) stops it and the rest is written from memory, so a save
  never fails because of it. Misaligned ranges (64 KiB clusters against the format's 4 KiB blocks)
  are not cloned.
- Orphaned temporaries use the same tags; the owner is checked with `process_running`.
- Journal temporaries are created hidden, so the journal is too. A journal never takes the model's
  read-only attribute, since Windows refuses to rename over or delete read-only files.
- Locks are mandatory (`LockFileEx`): only the holder reads a locked journal, through its own
  handle. Journals are renamed over while held open and locked, which needs NTFS POSIX rename
  semantics; on a volume without them the adjacent journal fails and the recovery directory takes it.
- Fallback journals (`file-<hash>.journal`) and journal markers hash the path's identity
  (`os::journal_identity`): final path of the file, else of its folder plus the name, else the
  absolute spelling, lowercased, so `C:\A\m.caditor` and `c:\a\M.caditor` share one journal.
- Paths stored as bytes (journal header, recovery markers, recent files) are WTF-8 from
  `as_encoded_bytes`, decoded with a safe decoder, so unpaired surrogates survive.

## App

- File dialogs are rfd's (`portal/windows.rs`), owned by the main window once it exists
  (`portal::own_dialogs`). rfd's `common-controls-v6` feature stays off: it imports
  `TaskDialogIndirect`, which unmanifested test binaries cannot load.
- `crash.rs` flushes the journal and ends the log on a console close and when Windows ends the
  session.
- After the first save (`Files::saved_folder`), `defender.rs` shows once a tip-like card saying
  Microsoft Defender's real-time scanning slows the atomic saves, journal and kept versions, how to
  exclude the models' folder and what that trades away; Read opens the `windows-defender` guide
  page. Either choice records `onboarding.defender_reminded`. Preferences › General › Saving on
  Windows (`defender::ON_WINDOWS`) shows it again (`PreferencesCommand::ShowDefenderReminder`).
  caditor never reads or changes Defender's settings.
- Backends: Direct3D 12, then Vulkan, then OpenGL (`caditor-render` `BACKEND_ORDER`).
- The built-in title bar drags with `StartDrag`, so Aero Snap works. The maximize button's rectangle
  goes to `set_maximize_button` after each frame (`Session::follow_maximize_button`); the reported
  `ButtonState` is kept in `window_frame::NonClientPointer` and requests a redraw so egui draws the
  button hovered or pressed although the pointer is outside the client area (its tooltip does not
  show).
- `build.rs` builds `caditor.ico` from the committed PNG renders and embeds it with
  `packaging/windows/caditor.exe.manifest` (PerMonitorV2, long paths, Common Controls v6) and
  version information through `winresource`, using `rc.exe`, or `llvm-rc` when cross-compiling.

## Installer (`packaging/windows/`)

- `caditor.wxs` is WiX 5 (MS-RL; later versions carry the OSMF EULA). A per-user MSI with no wizard:
  `%LOCALAPPDATA%\Programs\caditor` with `caditor.exe`, `README.md` and `licenses\` (the licence and
  Inter's), a Start menu shortcut and the `.caditor` association (`caditor.model`), all under HKCU.
  Each component's key path is an HKCU value; WiX cannot derive a GUID for a multi-file component
  that way, so `Program` and `Licences` have fixed ones. Those GUIDs and the `UpgradeCode` never
  change; same-version upgrades are allowed so a snapshot installs over itself.
- `build-release.ps1 [-Snapshot]` mirrors `build-release.sh`, writing
  `target/dist/caditor-<label>-windows-x86_64.msi` with its `.sha256`; `check-install.ps1` installs
  it quietly, checks files, `--version`, shortcut and association, installs over it, uninstalls and
  checks nothing is left.
