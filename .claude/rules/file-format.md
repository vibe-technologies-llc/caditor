---
paths:
  - "crates/caditor-file/**"
---

# File format and persistence

- Binary, zstd-compressed, xxh3-checked; no text model format. Every shipped `FORMAT_VERSION` stays
  readable forever.
- Evolving it:
  - Something whose loss changes what the model computes (suppression, the rollback bar) is a
    record kind of its own, not a feature field, so an older reader reports it as from a newer
    version instead of silently ignoring it.
  - A feature kind older readers lack gets its own record kind, but a feature the older kind can
    express is still written as it, so old readers keep reading it.
  - Optional derived data (`region_references`, `copies`, `origins`) is a field written only when
    known; older readers drop it and lose only the fallback it gives.
  - Anything else that must not be lost needs a must-understand chunk.

## Container (`binary/`)

- 8-byte magic (models, journals), LE `u32` version, chunks. Chunk: marker `CDCK`, kind, codec
  (stored, zstd, or zstd against the next newer version as raw prefix), flags (`MUST_UNDERSTAND`,
  `CONTINUED`, `CONTINUATION`; unknown bits ignored), lengths, xxh3-64 of header and payload.
- A bad chunk makes the reader scan to the next marker with a valid checksum; hashed bytes are
  capped (`HASHING_ALLOWANCE`) so forged headers cannot make that quadratic.
- A model with damaged magic or version is salvaged (`binary::salvage`) when a head chunk passing
  its checksum starts within `SALVAGE_REACH`: it loads with the version unknown, reported as a
  damaged start, and keeps its `.damaged` copy on saving. Anything else, a journal included, is
  `LoadError::NotAModel`.
- Content over `MAX_CONTENT` is sliced, each slice its own frame against the same prefix (all but
  the last `CONTINUED`, all but the first `CONTINUATION`); the parser joins a run up to
  `MAX_JOINED_CONTENT` (`Chunk::unpack`). A run broken by damage or another kind is one damaged
  piece, so a resync mid-run never yields partial content. Older readers see only the first slice,
  which fails its digest.
- Memory is bounded on hostile files: records decode one at a time (unpack, hash with
  `RecordDigest` checked against the head afterwards, parse); a load unpacks at most
  `MAX_DECOMPRESSED` in all (`Budget`); a version rebuild bounds what is alive per step instead
  (`unpack_beside`), so long histories of large snapshots still restore. `Budget::unpack` keeps the
  `UnpackError`: an unknown codec reads like content from a newer version, running out of memory or
  budget says too large to load, only the rest is "damaged".
- A save holds the model twice at most, uncompressed (`PriorRecords`, `NewRecords`).

## Values (`binary/value.rs`)

- Self-describing serde encoding with a nesting limit; enums like JSON.
- Only finite floats are encoded: NaN or infinity fails as `ValueError::NonFinite` and the save is
  refused with a reason, since the lenient reader would turn it into `Null` and load the record as
  damaged.
- `Lenient` reads a record through a `serde_json::Value`: unknown fields ignored, unknown record
  kinds reported as from a newer version.

## Model file

- Head chunk (save time, last change's name, blake3 digest of records), one zstd chunk per record
  (`RECORD_KINDS`), version history. The journal snapshot carries `principal`, `suppressed` and
  `rollback` as fields.
- A record with unchanged understood content is written back exactly as stored (newer fields kept);
  old records are matched by the digest of their re-encoded understood content, not their bytes.
  Unknown chunk kinds are carried unless must-understand; loading reports those as left out, so the
  original is kept as `.damaged`.
- A parameter record's `note` is written only when not empty, like a feature's `hidden`; an older
  reader drops it. A note too long for this version is cut at `MAX_PARAMETER_NOTE_CHARS`,
  reported.
- Records carry stable IDs. Expressions are canonical text with parameters as `$<id>`
  (`to_stored_text`, `parse_stored`); region keys and topology names are 32-digit hex; numbers
  exact f64.
- An import is an `import` record (source name, STEP text); one that cannot be read loads empty,
  reported. Parsing goes through `step_cache.rs`, a process-wide LRU cache of solids by the blake3
  digest of the text, so a load, a journal replay and a recovery scan parse each import once.
- `ValueError::Refused` is what serde's `custom` becomes and carries no message: no type of ours
  raises it, only serde's own derives.
- Unreadable values become reported fallbacks (the `restore_*` functions in `format.rs`), never
  refuse the file: each says what was set to what and keeps the feature.

## Version history

- Each save that changes the model keeps the state it replaces as a version in the file
  (`save_with` reads earlier ones from the session's file, so Save As carries them).
- Version: info chunk (time, last change, blake3 digest) plus data chunk compressed with the next
  newer version as zstd prefix; every `KEYFRAME_SPACING`th stored whole, bounding a damaged chunk's
  chain. Info belongs only to the data right after it: data with damaged info is kept (unlisted) so
  older deltas decode; info with damaged data is dropped. Rebuilt versions are checked against
  their digest before restoring.
- `history` judges rebuildability from the chain rather than by decoding (`Parsed::rebuildable`), so
  listing an undamaged file unpacks at most the head's records. A version that passes its checksum
  but does not rebuild to its digest is listed and refused on restore. When the head cannot be
  rebuilt, the next save drops the deltas that depended on it.
- Retention (`binary/retention.rs`), on each save adding a version: the newest stay, older thin by
  age tiers; unlisted always stay. A delta whose newer neighbour was dropped is recompressed
  against the newest kept version before it (stored whole when a dropped version was whole); one
  that cannot be decoded is copied as it was. When a delta could not be decoded for want of memory
  the versions dropped before it are kept, so thinning never leaves a delta whose newer neighbour
  is gone; if a second pass still runs short the save fails (`EncodeError::HistoryTooLarge`) rather
  than write such a delta.
- Size: a model file stays within the `MAX_FILE_SIZE` every reader enforces. When versions would
  take it past three quarters of that, the oldest are dropped (a tail, so nothing depends on them)
  and the save says how many (`Encoded::dropped_for_size`). A model too large on its own is refused
  (`EncodeError::ModelTooLarge`), since it could not be opened again, and so are records whose
  uncompressed snapshot exceeds `MAX_DECOMPRESSED` (exported as `MAX_MODEL_RECORDS`), checked
  before anything is written rather than found when the read-back runs out of budget.
- Versions copied unchanged are block-aligned (a zero-filled `Padding` chunk, skipped by readers)
  so `copy_file_range` (`os/unix.rs`) can clone them; what could not be cloned (everything on
  Windows), or the file having changed since it was read, is written from memory.

## Saving

- Write a temporary sibling, fsync, rename over the target, fsync the directory (a failed directory
  fsync is logged, not a failed save). On Windows the rename is `ReplaceFileW` and there is no
  directory fsync (`windows.md`). Permissions, group and extended attributes (ACLs too) are
  kept; the temporary is owner-only until applied. A symbolic link is followed.
- Temporaries are `.<name>.<machine>-<boot>-<pid>-<n>.tmp`. After each write, those next to the
  file are removed when from an earlier boot of this machine, or from this boot with their process
  gone; other machines' saves on a shared folder are left alone. The recovery-directory scan
  sweeps the same way.
- Names past `MAX_NAME_BYTES` (temporaries, `.damaged` copies) are cut and end in a hash
  (`paths::fitting`); a model whose name leaves no room for `.<name>.journal` keeps its journal in
  the recovery directory.
- `SaveOptions::unless_changed_from` (the head digest the session loaded or last saved) makes a
  save refuse with `SaveError::ChangedOnDisk` when the file it replaces, being the one its
  versions are read from, now has another head, leaving it as it was. Replacing anyway makes the
  outside head the newest version.
- A save refuses when the earlier versions in the file it replaces cannot be read, and when the
  file is read-only (since `rename` would replace it anyway); the notice offers Save As.
- Before the rename, a model save reads the synced temporary back and checks that it parses
  undamaged, its head digest is the one encoded and every record hashes to it
  (`binary::reads_back`); otherwise the temporary is removed and the file left as it was. This
  guards against the compressor and the shared-range copies.
- Overwriting a file that loaded with problems first keeps the original as
  `<name>.damaged.caditor` (`keep_copy`, also used for unreadable preferences and recent files): a
  hard link, else a copy moved into place whole. The same copy is kept when the file being replaced
  has gone bad since it was loaded (`Encoded::previous_damaged`), since the save drops what it
  cannot read.

## Reading and loading

- Every file caditor reads goes through `read.rs`: regular files only, at most `MAX_FILE_SIZE`;
  devices, pipes and huge sparse files are refused in words. Saving over a non-regular file is
  refused.
- An extrusion record (`extrude` and `extrude_to`) carries `start`, the stored text of its start
  offset, only when it has one. A start at a face or plane, and any start of a revolution, change
  what the model computes, so they are record kinds of their own (`extrude_from`, `revolve_from`):
  their `start` is `distance` with its text or `plane` with a plane reference, and an unreadable
  one loads as starting at the sketch plane (or 0 mm), reported.
- A `hole` feature record holds `sketch`, `body`, `diameter`, `depth` (`through_all` or
  `blind` with its text), `style` (`plain`, or `counterbore` or `countersink` with their texts) and
  `reversed` when set; an unreadable size loads as a default (5 mm, 10 mm, 3 mm, 90 deg), reported.
- A `move` feature record holds `body` and the stored text of its three distances (`offset`) and
  three turns (`turn`); an unreadable one loads as 0 mm or 0 deg, reported.
- A `mirror` feature record holds `body`, `plane` (a plane reference; an unreadable one loads as
  the YZ plane, reported) and `keep_original`. A `scale` record holds `body`, the stored text of
  `factor` (unreadable: 1) and of the three `center` lengths (unreadable: 0 mm).
- A feature record carries `appearance` only when a body has one: `colour` as `#rrggbb`,
  `material`, and `density` as stored text, each optional. Losing it changes nothing computed, so
  it is a field, not a record kind; an unreadable part is left out and reported, the rest kept.
  The journal's `set_body_appearance` holds the same record. `describe_unreadable_record` skips
  the feature fields (`FEATURE_FIELDS`) when naming an unknown kind.
- A `combine` feature record holds `body`, `tool` and `operation` (`join`, `cut`, `intersect`).
- A sketch's constraint record carries `inactive: true` only for a disabled constraint (absent
  means active, so older files read unchanged); the journal's `add_sketch_constraint` carries the
  same flag and `set_sketch_constraint_active` is its own record.
- A sketch record carries `projections` only when it has projected geometry: each the projected
  entity's `entity` and its `source` (`edge` with `body` and an edge record, `vertex` with `body`
  and the vertex name's digest, or `sketch_entity` with `sketch` and `entity`). The projected flags
  of the entity and its points are not stored; loading derives them from this list after the
  constraints, so stored constraints between projected geometry still load. An unreadable source
  leaves its geometry as ordinary geometry where it was saved, reported. The journal's
  `set_sketch_projection` holds the same source, absent for none.
- Loading is partial: each record and sketch item is read on its own (`Lenient`) and assembled
  through `Document::apply`, so a loaded model satisfies the document invariants.
- Reported repairs: damaged or unknown records left out; a lost parameter still used becomes a
  stand-in of value 0; unusable or duplicate names renamed; a parameter cycle broken at the
  parameter closing it; an unreadable dimension takes its drawn length; a rollback bar above an
  absent feature goes to the end. Suppressed IDs of absent features are dropped silently.
- A feature referring to a feature absent from the file loads as saved, since deleting a feature
  may keep its dependents (`document.md`); it fails when computed, saying what it lost.
- Records not matching the head's digest with no damaged chunk (cut between chunks) are reported as
  ending early; the model loads with problems and keeps its `.damaged` copy on the next save.
- `load` and `load_version` (not `decode`, which fuzzing and recovery scans use) complete the
  origins of face and edge references saved without them (`complete_origins`, `document.md`)
  within `ORIGIN_COMPLETION_TIME`; the completed model is the saved baseline, so it opens
  unmodified and the next save writes the origins. A journal snapshot is not completed.
- Near-linear on hostile files: names indexed; duplicate IDs and cycles (`DependencyGraph`) found
  before applying; each kind of record applied as one transaction, halved only where it fails; at
  most `MAX_RECORDS` parameters and features loaded, the rest reported.
