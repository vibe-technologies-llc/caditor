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
  change (`apply`, `undo`, `redo`). Replay stops at the first damaged chunk (a torn tail loses only
  later changes) and goes through an `Editor`, so undo history is restored.
- Undo and redo are journaled by reference when they can be (`UndoLast`, `RedoNext`), so undoing an
  import never repeats its STEP text: the storage worker follows the journal in a `Mirror`. Anything
  the mirror cannot follow is written whole, and the mirror stops until the next save. Replay
  resolves references itself, so `Recovered::entries` hold whole transactions.
- Rebase: once the bytes appended since the last rewrite exceed both
  `StorageConfig::rebase_journal_after` and that rewrite's size, the mirror's document becomes the
  snapshot, its entries are counted in `folded` and dropped, and `Report::Rebased` hands the app the
  new base. The undo steps of folded changes are not recovered.
- A rebased snapshot is a `RebasedSnapshot` chunk, never a plain `Snapshot`: it is not the saved
  state, so the scan must not delete it as unchanged, and an older caditor, not knowing the kind,
  reads the journal as damaged and sets it aside instead of deleting it. A recovered session with
  folded changes has no known saved state (`Model::saved` is `None`), so it stays modified.
- The header carries the path as raw bytes (`os::path_bytes`: Unix bytes, WTF-8 on Windows, so
  names that are not Unicode survive), whether the file loaded with
  problems (a recovered session still keeps the damaged original on its first save), and `on_disk`,
  the head digest of the file last loaded or saved, so a recovered session still notices an outside
  change. The worker passes it as `SaveOptions::unless_changed_from` unless the request says
  `replace_outside_changes`, and reports `Report::ChangedOnDisk`.
- New edit kinds do not bump `JOURNAL_VERSION`: an older reader stops at the first entry it cannot
  read, keeping all before.
- Lives at `.<name>.journal` next to the file, else under `recovery_dir` (untitled documents, or a
  name too long). Takes the model's permissions, owner-only when untitled. On Windows it is hidden
  and never read-only (`windows.md`).

## Locks and placement (`lock.rs`)

- The owner holds an exclusive lock, which is how the scan and other instances tell live from
  orphaned; read-only journals get a shared one. Taken on read-write handles, since NFS emulates
  `flock` with byte-range locks needing a writable descriptor.
- Windows locks are mandatory: only the holder can read a locked journal, through its own handle.
  Identity is `os::FileKey` (dev and inode on Unix, `FileId` on Windows).
- A spawned child shares locked descriptors until it execs, so unit tests never spawn processes;
  the only test that does is the app's `tests/crash_flush.rs`, in a binary of its own.
- A journal is put in place only by linking where none exists, or renaming over the inode the
  writer holds locked and has checked is still at that path, so two windows cannot take one path
  from each other. The owner rechecks the path after every sync, else reports itself unprotected
  and retakes it when it can. Saving to another path is refused while another window holds that
  model's journal.

## `Storage`

- One worker thread per open document; owns the journal and performs saves, keeping appends, saves
  and rebases in order. Fsyncs after each batch.
- After a write error (`Report::JournalFailed`) it keeps the lock, stops appending and rewrites the
  whole journal every `RETRY_INTERVAL` until it succeeds (`Report::JournalRestored`).
- A replaced journal is removed only once the new one is written.
- A `Flusher` lets the panic hook and the termination handlers (signals, a closing console, the
  end of a Windows session) wait for pending entries.

## Preferences and recent files (`settings.rs`, `recent.rs`)

- `Settings`: JSON key/value `preferences.json` in `config_dir`, read leniently (`load` falls back
  to the defaults), written atomically; unknown keys are kept, so an older caditor never erases a
  newer one's settings. Recent files are the same design in the state directory; non-UTF-8 paths
  are stored as byte arrays.
- Windows save only what they changed (`save_changes`), re-reading the file under an advisory lock
  (`lock::locked_update`: polled up to `UPDATE_LOCK_WAIT`, then the update goes ahead unlocked with
  a warning rather than losing the change), so windows keep each other's changes.
- An unreadable file is kept as `<stem>.unreadable…` before it is replaced.

## Recovery (`recovery.rs`: `scan`, `journal_for`)

- Inspects unlocked journals in the recovery directory, next to recent files and wherever a marker
  names one: a journal next to its model leaves an `adjacent-<hash>.location` marker in the
  recovery directory, removed with the journal, so a crashed model's unsaved work is offered even
  when no recent file names it.
- The scan deletes journals with nothing to recover (no net change or already in the file, and
  every entry read) and returns the rest with a replayed `Editor`; one with unreadable entries
  (damaged, or newer) is offered, never deleted. Before deleting, the locked file is compared with
  the path by inode, so a journal an owner renamed into place meanwhile is left alone.
- A journal of an unreadable model (bad header or snapshot, or newer journal version) is renamed
  aside by `journal_for` to `<journal>.<seconds>.unreadable` before the new journal takes its
  place (`FileJournal::SetAside`), and opening reports where. The scan removes those after
  `paths::SET_ASIDE_KEPT_SECONDS`; until then it inspects them like any journal, so the newer
  caditor that wrote them offers them for restoring, and `journal_for` offers the newest readable
  one when its own journal holds nothing (`Start::replaces`).

## Session logs (`logs.rs`)

- `SessionLog` writes an owner-only, append-only log per session in the state directory, stopping
  at `MAX_LOG_SIZE`; `ended_unexpectedly` lists other logs whose process is gone and whose last
  line is neither the end nor the reported line (`os::process_running`: `/proc` on Unix,
  `OpenProcess` on Windows); `prune_logs` keeps the `LOGS_KEPT` newest and
  never removes a running process's.
