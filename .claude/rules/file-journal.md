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
---

# Journal, storage, preferences and recovery

## Recovery journal

- Model container (`file-format.md`): header chunk naming the file, snapshot of the last saved
  state, one chunk per change (`apply`, `undo`, `redo` with the transaction).
- Undo and redo are journaled by reference when they can be: the storage worker follows the
  journal in a `Mirror` (an `Editor` from the snapshot, fed every entry as replay would), and an
  undo or redo whose transaction is the mirror's next one is written as an empty `UndoLast` or
  `RedoNext` chunk, so undoing and redoing an import never repeats its STEP text. One reaching
  past the snapshot (undoing beyond a save or a rebase) is written whole, as is everything once
  the mirror cannot follow (it then logs a warning and stops until the next save). Replay resolves
  the references through its own `Editor`, so `Recovered::entries` hold whole transactions.
- Past a size the worker rebases the journal in the background: once the bytes appended since the
  last rewrite exceed both `StorageConfig::rebase_journal_after` (32 MiB) and that rewrite's size,
  the mirror's document becomes the snapshot, its entries are counted in `folded` and dropped, and
  the journal is rewritten whole; `Report::Rebased` hands the app the new base and the count so it
  drops its own copies (`Model::journal_base`, `folded`). A journal stays within about twice the
  larger of the threshold and the model, and the worker's and app's entries within the threshold.
  The undo steps of folded changes are not recovered.
- A rebased snapshot is a `RebasedSnapshot` chunk (`{snapshot, folded}`), never a plain
  `Snapshot`: it is not the saved state, so the scan must not delete it as unchanged, and an older
  caditor, not knowing the kind, reads the journal as damaged and sets it aside rather than
  deleting it. `Recovered::folded` and `Start::folded` carry the count; `changes()` includes it.
  A recovered session with folded changes has no known saved state (`Model::saved` is `None`), so
  it stays modified until saved.
- Header carries the path as raw bytes (non-UTF-8 names survive), whether the file loaded with
  problems, so a recovered session still keeps the damaged original on its first save, and
  `on_disk`, the head digest of the file the session last loaded or saved (written only when
  known; older readers ignore it), so a recovered session still notices an outside change.
- The storage worker keeps `on_disk` from `Start`, passes it as `SaveOptions::unless_changed_from`
  unless the request says `replace_outside_changes`, reports a mismatch as
  `Report::ChangedOnDisk` and takes the new digest from each save (`Report::Saved::digest`).
- Replay stops at the first damaged or unreadable chunk (a torn tail loses only later changes);
  through an `Editor` it restores undo history.
- New edit kinds do not bump the journal version: an older reader stops at the first entry it
  cannot read, keeping all before. `set_feature_suppressed`, `set_rollback_bar` (`before`, absent
  for the end) and `insert_feature`'s `suppressed` field (written only when set) came with model
  format 3; the snapshot's optional `suppressed` and `rollback` fields hold the records of the
  same names.
- Lives at `.<name>.journal` next to the file, else `$XDG_STATE_HOME/caditor/recovery/` (untitled
  documents). Takes the model's permissions, owner-only when untitled.

## Locks (`lock.rs`)

- The owner holds an exclusive lock: how the startup scan and other instances tell live from
  orphaned. Taken on read-write handles (read-only journals get a shared lock), since NFS emulates
  `flock` with byte-range locks needing a writable descriptor.
- A spawned child shares locked descriptors until it execs, so a scan meanwhile sees a live journal
  as locked; unit tests never spawn processes (FIFOs are made in-process through `rustix`). The
  only test that does is the app's `tests/crash_flush.rs`, in a binary of its own.

## Putting a journal in place

- Only by linking where none exists, or renaming over the inode the writer holds locked and has
  checked is still at that path; two windows cannot take one path from each other.
- The owner rechecks the path after every sync, else reports itself unprotected and retakes it when
  it can.
- Saving to another path is refused while another window holds that model's journal; a journal is
  never renamed over one another window holds.

## `Storage`

- One worker thread per open document; owns the journal and performs saves, keeping appends, saves
  and the journal's rebase onto the saved snapshot in order. Fsyncs after each batch.
- Keeps the journal's snapshot (the saved state, or the last rebase) and entries since. After a write error (`Report::JournalFailed`) it
  keeps the lock, stops appending and rewrites the whole journal every few seconds until it
  succeeds (`Report::JournalRestored`).
- A replaced journal (after a save or restore) is removed only once the new one is written; a
  failed directory fsync after the rename is logged, not a failed write.
- A `Flusher` lets the panic hook and signal handler wait for pending entries.

## Preferences (`settings.rs`)

- `Settings`: JSON key/value file `$XDG_CONFIG_HOME/caditor/preferences.json` (`config_dir`), read
  leniently (`load` falls back to the defaults, `load_reporting` also returns the `SettingsError`
  that made it), written atomically; unknown keys are kept, so an older caditor never erases a newer
  one's settings.
- `save_changes` re-reads the file and applies only keys changed since the last save, so windows
  keep each other's changes. An unreadable file is kept as `preferences.unreadable.json` first.
- The read-modify-write runs under an advisory `flock` on `preferences.lock` beside the file
  (`lock::locked_update`: polled for up to three seconds, then the update goes ahead unlocked with
  a warning rather than losing the change), so two windows saving at once lose nothing.

## Recent files (`recent.rs`)

- Non-UTF-8 paths are stored as byte arrays. Windows write `RecentChange`s applied to what is on
  disk (`RecentFiles::save_changes`), keeping each other's entries, under the same kind of lock on
  `recent-files.lock`.
- An unreadable list is kept as `recent-files.unreadable.json` (`-2`, `-3` … when taken) before
  it is replaced, in `save_changes` and `save` alike.

## Recovery (`recovery.rs`: `scan`, `journal_for`)

- Inspects unlocked journals in the recovery directory, next to recent files and wherever a marker
  names one. A journal next to its model leaves an `adjacent-<hash>.location` file in the recovery
  directory holding its absolute path, removed with the journal (the scan prunes it once the
  journal is gone), so a crashed model's unsaved work is offered even when no recent file names it.
- The scan deletes journals with nothing to recover (no net change or already in the file, and
  every entry read) and returns the rest with a replayed `Editor`. One with unreadable entries
  (damaged, or newer) is offered, never deleted.
- Before deleting, the locked file is compared with the path by inode, so a journal an owner
  renamed into place meanwhile is left alone.
- A journal of an unreadable model (bad header or snapshot, or newer journal version) is renamed
  aside by `journal_for` to `<journal>.<seconds>.unreadable` (no scan picks it up) before the new
  session's journal takes its place (`FileJournal::SetAside`); opening reports where it was kept.
  The scan removes those in the recovery directory and next to recent files once the seconds in
  their name are over thirty days old (`paths::SET_ASIDE_KEPT_SECONDS`). Until then the scan
  inspects them like any journal, so once a caditor that reads them runs (the newer version that
  wrote them), they are offered for restoring, and `journal_for` offers the newest readable one
  set aside for a model when its own journal holds nothing; restoring takes it over like any
  recovered journal (`Start::replaces`), and one with nothing left to recover is deleted.

## Session logs (`logs.rs`)

- `SessionLog::create` makes `<state>/logs/caditor-<seconds, 20 digits>-<pid>.log`, owner-only,
  append-only; `write` stops at `MAX_LOG_SIZE` with one line saying so, `end` appends the end
  line past that limit and syncs. `ended_unexpectedly` lists other logs whose process is gone
  (`/proc/<pid>`) and whose last line (read from the last 512 bytes) is neither the end line nor
  the reported line; `mark_reported` appends the reported line; `prune_logs` keeps the
  `LOGS_KEPT` newest by name and never removes a running process's.

