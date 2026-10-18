---
paths:
  - "crates/caditor-file/src/journal.rs"
  - "crates/caditor-file/src/lock.rs"
  - "crates/caditor-file/src/storage.rs"
  - "crates/caditor-file/src/settings.rs"
  - "crates/caditor-file/src/recent.rs"
  - "crates/caditor-file/src/recovery.rs"
  - "crates/caditor-file/src/paths.rs"
  - "crates/caditor-file/src/logs.rs"
  - "crates/caditor-file/src/os/**"
---

# Journal, storage, preferences and recovery

## Recovery journal

- Model container (`file-format.md`): header, snapshot of the last saved state, one chunk per
  change (`apply`, `undo`, `redo`). Replay stops at the first damaged chunk and goes through an
  `Editor`, so undo history is restored.
- `JOURNAL_VERSION` is 1. Decoding fails as `UnreadableJournal`: `Damaged`, `FromNewerVersion`, or
  `FromEarlierBuild` for a journal of a development build before version 1 (magic
  `\x89CJL\r\n\x1a\n`). Each is handled alike: never offered, set aside by `journal_for`.
  Every shipped version keeps journals in the format corpus (`file-format.md`).
- The journal carries no preview picture (`file-format.md`): a picture pictures a saved file, and a
  journal restores unsaved work, so the file written when a recovered session is saved draws its
  own. `SaveRequest::preview` reaches only the model file; the worker hands it to `SaveOptions` and
  the journal rewrite after the save is untouched by it. The worker syncs the journal before a save
  and waits for the picture inside the save (at most `PREVIEW_PATIENCE`, only while the drawing is
  still running), so appends queue behind it for that long at most.
- Undo and redo are journaled by reference when possible (`UndoLast`, `RedoNext`), so undoing an
  import never repeats its STEP text; the storage worker follows the journal in a `Mirror`.
- A reference image's picture is written into a journal once: the worker keeps the digests the file
  holds (`WrittenPictures`: the base's pictures, set by each rewrite in `encode_journal_noting`,
  added to by each append in `encode_entry_sharing`), and a journaled `insert_feature` or
  `set_feature_kind` of a picture already there holds only its digest. Decoding gives entries the
  pictures of the base and of the entries before them (`restore_transaction_with`), so changing an
  image's opacity journals a few bytes, not the picture.
  Anything the mirror cannot follow is written whole and the mirror stops until the next save.
  Replay resolves references, so `Recovered::entries` hold whole transactions.
- Rebase: once bytes appended since the last rewrite exceed both `StorageConfig::rebase_journal_after`
  and that rewrite's size, the mirror's document becomes the snapshot, its entries are counted in
  `folded` and dropped, and `Report::Rebased` hands the app the new base. Undo steps of folded
  changes are not recovered.
- A rebased snapshot is a `RebasedSnapshot` chunk, never a plain `Snapshot`: it is not the saved
  state, so the scan must not delete it as unchanged. A recovered session with folded changes has
  no known saved state (`Model::saved` is `None`), so it stays modified.
- A plain snapshot is the saved state, so one failing its checksum does not withhold later
  entries: when the header says nothing was folded (`folded` is 0) and the model file's head
  digest equals the header's `on_disk`, `decode_journal_with` takes the file as the base
  (`recovery::model_as_last_saved`), replays the entries onto it and adds an issue. A damaged `RebasedSnapshot`, or a plain one whose model changed, is treated as
  unreadable (below).
- The header carries the path as raw bytes (`file`, `os::path_bytes`: Unix bytes, WTF-8 on
  Windows), whether the file loaded with problems (a recovered session still keeps the damaged original on
  its first save), and `on_disk`, the head digest of the file last loaded or saved, so a recovered
  session notices an outside change. The worker passes it as `SaveOptions::unless_changed_from`
  unless the request says `replace_outside_changes`, and reports `Report::ChangedOnDisk`.
- Chunks are read only after a crash, so they are packed at zstd `Level::FAST`
  (`push_packed_for_recovery`), about 4x quicker than the saved model's `Level::BALANCED` for some
  30% more bytes. Frames do not record the level, so either reads alike.
- New edit kinds do not bump `JOURNAL_VERSION`: an older reader stops at the first entry it cannot
  read, keeping all before.
- Lives at `.<name>.journal` next to the file, else under `recovery_dir` (untitled documents, or a
  name too long). Takes the model's permissions, owner-only when untitled. On Windows it is hidden
  and never read-only (`windows.md`).

## Locks and placement (`lock.rs`)

- The owner holds an exclusive lock, which is how the scan and other instances tell live from
  orphaned; read-only journals get a shared one. Taken on read-write handles, since NFS emulates
  `flock` with byte-range locks needing a writable descriptor.
- Windows locks are mandatory: only the holder reads a locked journal, through its own handle.
  Identity is `os::FileKey` (dev and inode on Unix, `FileId` on Windows).
- A spawned child shares locked descriptors until it execs, so unit tests never spawn processes;
  the one that does is the app's `tests/crash_flush.rs`, in a binary of its own.
- A journal is put in place only by linking where none exists, or renaming over the inode the
  writer holds locked and has checked is still at that path, so two windows cannot take one path
  from each other. The owner rechecks the path after every sync, else reports itself unprotected
  and retakes it when it can. Saving to another path is refused while another window holds that
  model's journal.

## `Storage`

- One worker thread per open document owns the journal and performs saves, keeping appends, saves
  and rebases in order; fsyncs after each batch.
- After a write error (`Report::JournalFailed`) it keeps the lock, stops appending and rewrites
  the whole journal every `RETRY_INTERVAL` until it succeeds (`Report::JournalRestored`). A
  replaced journal is removed only once the new one is written.
- A worker that stops (a command or a poll finds its channels closed) is replaced by the app
  (`Model::restart_storage`) with the model's base and every entry since it, so no entry is lost;
  a save in flight is reported as failed. An untitled document's new worker names a new journal
  and removes the dead worker's (`Storage::untitled_journal`, `Start::replaces`), so the stop
  leaves one journal to recover, not two. The `test-support` feature adds `Storage::stop`, which
  the app's tests enable as a dev-dependency, to stop a worker on command.
- A worker started for another model (`Start::after`, the previous worker's `Closing`) waits up to
  `PREDECESSOR_TIMEOUT`, then completes its `Handover` (`Storage::handover`); anything inspecting
  journals for that model's file waits on it first, since until then the previous worker still
  holds the lock and the file would be judged open in another window.
- A fallback journal is named by a hash of the model's path identity (`os::journal_identity`: the
  path as is on Unix, the case-folded final path on Windows); `paths::journals_for` gives the
  adjacent journal and that one, and a journal's marker is named the same way (`journal_marker`).
- A `Flusher` lets the panic hook and the termination handlers (signals, closing console, end of
  a Windows session) wait for pending entries.

## Preferences and recent files (`settings.rs`, `recent.rs`)

- `Settings`: JSON `preferences.json` in `config_dir`, read leniently (`load` falls back to
  defaults), written atomically; unknown keys are kept, so an older caditor never erases a newer
  one's settings. Recent files are the same design in the state directory; non-UTF-8 paths are
  stored as byte arrays. A file is listed once, compared with `os::same_file_path` (exact on Unix;
  by component, ignoring case on Windows), the latest spelling kept.
- Windows save only what they changed (`save_changes`), re-reading the file under an advisory lock
  (`lock::locked_update`: polled up to `UPDATE_LOCK_WAIT`, then the update proceeds unlocked with a
  warning rather than losing the change), so windows keep each other's changes.
- An unreadable file is kept as `<stem>.unreadable...` before it is replaced.
- A failed update is an `UpdateFailure`: `Reading` (`ReadFailure`) for the file merged into,
  `Writing` (`WriteFailure`, `FolderIsAFile` when a file stands where the folder should be) for
  the folder, kept copy and write, `Encoding` for the JSON, so the app words it.

## Recovery (`recovery.rs`: `scan`, `journal_for`)

- Inspects unlocked journals in the recovery directory, next to recent files and wherever a marker
  names one: a journal next to its model leaves an `adjacent-<hash>.location` marker in the
  recovery directory, removed with the journal, so a crashed model's unsaved work is offered even
  when no recent file names it.
- The scan deletes journals with nothing to recover (no net change or already in the file, every
  entry read) and returns the rest with a replayed `Editor`; one with unreadable entries (damaged,
  or newer) is offered, never deleted. Before deleting, the locked file is compared with the path
  by inode, so a journal an owner renamed into place meanwhile is left alone.
- `discard` removes a journal no window holds, comparing by inode the same way: already gone is
  `Ok`, held or replaced by another window is `DiscardError::InUse`, other failures are `Reading`
  or `Removing` with the worded `ReadFailure` or `WriteFailure`.
- A journal of an unreadable model (bad header or snapshot, a newer journal version, or one from an
  earlier build) is renamed by `journal_for` to `<journal>.<seconds>.unreadable` before the new
  journal takes its place (`FileJournal::SetAside`), and opening reports where. The scan removes those after
  `paths::SET_ASIDE_KEPT_SECONDS`; until then it inspects them like any journal, so the newer
  caditor that wrote them offers them for restoring, and `journal_for` offers the newest readable
  one when its own journal holds nothing (`Start::replaces`).

## Session logs (`logs.rs`)

- `SessionLog` writes an owner-only, append-only log per session in the state directory, stopping
  at `MAX_LOG_SIZE`; `ended_unexpectedly` lists other logs whose process is gone and whose last
  line is neither the end nor the reported line (`os::process_running`); `prune_logs` keeps the
  `LOGS_KEPT` newest and never removes a running process's.
