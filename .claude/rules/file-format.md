---
paths:
  - "crates/caditor-file/**"
---

# File format and persistence

- Binary, zstd-compressed, xxh3-checked; no text model format. Format number restarted at 1; every
  shipped version stays readable. Version 2 added `linear_pattern` and `circular_pattern`
  feature records, which version 1 readers leave out as a kind they do not know.
- Version 3 adds four records, each written only when needed. `suppressed` (`{"features": [ids]}`,
  the suppressed features) and `rollback` (`{"before": id}`, the first feature below the rollback
  bar): losing either changes what the model computes, so they are records of their own rather
  than feature fields, and a version 2 reader reports them as unknown records from a newer version
  and computes every feature. Two feature records: `extrude_to`, an extrusion with a side that is
  not a distance (`extent` is `{"one_side": {"end", "reversed"}}` or
  `{"two_sides": {"forward", "backward"}}`, each end `{"distance": text}`, `"through_all"`,
  `"up_to_next"` or `{"up_to_face": plane reference}`), and `revolve_two_angles` (`forward` and
  `backward` angles beside the revolve's fields). An extrusion of distances alone is still an
  `extrude` record and a revolve of one or no angle a `revolve` record, so version 2 readers read
  them; the new kinds they leave out as a kind they do not know rather than misreading them.

## Container (`binary/`)

- 8-byte magic (`\x89CAD\r\n\x1a\n` models, `\x89CJL…` journals), LE `u32` version, chunks.
- Chunk: marker `CDCK`, kind, codec (stored, zstd, or zstd against the next newer version as raw
  prefix), flags (`MUST_UNDERSTAND`, `CONTINUED`, `CONTINUATION`; unknown bits ignored), reserved
  zero byte, stored and content lengths, xxh3-64 of header fields and payload, payload.
- A bad chunk makes the reader scan to the next marker with a valid checksum; hashed payload bytes
  are capped at four times the file size (forged headers cannot make it quadratic).
- A model whose magic or version is damaged is salvaged (`binary::salvage`) when a chunk passing
  its checksum starts at the first marker within the first 64 bytes and is a head chunk: it loads
  from its chunks with the version unknown, reported as a damaged start, and counts as damaged
  for saving (the `.damaged` copy is kept, its versions carried). Anything else, a journal
  included (its first chunk is a journal header), stays `LoadError::NotAModel`.
- Chunks hold at most 256 MiB; longer content is sliced that size, each slice its own frame against
  the same prefix: all but the last `CONTINUED`, all but the first `CONTINUATION`. The parser joins
  a run of one kind into one logical chunk of at most 2 GiB (`Chunk::unpack`); a run broken by
  damage or another kind, and a continuation with no run, is one damaged piece (a resync mid-run
  never yields partial content). Older readers see only the first slice, which fails its digest.
- Records decode one at a time: a load unpacks, hashes (`RecordDigest`, checked against the head
  afterwards) and parses each before the next, so only the parsed model accumulates. A load
  unpacks at most 2 GiB of records in all (`Budget`, cumulative, bounding the parsed model too); a
  version rebuild (restore, thinning) bounds what is alive at each step instead
  (`unpack_beside`: the snapshots held plus the one being decoded), so a long history of large
  snapshots still restores.
  `Budget::unpack` keeps the `UnpackError`, so a record in an unknown codec reads like content from
  a newer version, one that ran out of memory or the budget says it is too large to load, and only
  the rest is "damaged".
- A save holds the model twice at most, uncompressed: the replaced file's records are unpacked
  one at a time into its snapshot (`PriorRecords`), which the version delta needs whole, and
  the new records are encoded one at a time into theirs (`NewRecords`), each record kept or
  written fresh being a range of one of the two. The read-back check after writing streams its
  records through `RecordDigest` the same way.

## Values (`binary/value.rs`)

- Self-describing serde encoding: tagged null, booleans, LEB128 unsigned and negative integers, LE
  f64, strings, bytes, sequences and maps closed by an end tag; nesting limited. Enums like JSON
  (unit variant is its name, others a one-entry map).
- Only finite floats are encoded: NaN or an infinity fails as `ValueError::NonFinite`, and a save
  is refused with a reason saying so, since the lenient reader would turn it into `Null` and load
  the record as damaged.
- `Lenient` reads any record through a `serde_json::Value`: unknown fields ignored, unknown record
  kinds reported as from a newer version.
- A newer version adding something whose loss changes the model's meaning needs a new record kind
  or a must-understand chunk; older readers drop unknown fields of records they change.

## Model file

- Head chunk (save time, last change's name, blake3 digest of records), one zstd chunk per record,
  version history. Records: parameters, features, ID counters, `principal` (hidden principal
  planes, axes, origin; only when one is hidden), `suppressed` and `rollback`; the journal
  snapshot carries the last three as fields.
- A record with unchanged understood content is written back exactly as stored (newer fields kept,
  not recompressed); the old records are matched by the blake3 digest of their re-encoded
  understood content, not by those bytes. Unknown chunk kinds are carried unless must-understand; loading reports those
  as left out, so the original is kept as `.damaged`.
- An extrusion or revolve record with chosen regions has `region_references` beside `regions`,
  aligned with its keys: each region's `boundary` (`entity`, `side` `left` or `right`, `piece` id
  digest) and `anchor` (`[x, y]` on the sketch plane), written only when any is known; older
  readers drop it and lose only the fallback, and a key without one loads as a key alone.
- A face or attachment record whose face is a patterned copy has `copy` (`pattern`, `index`) beside
  `origin`, which holds the original's origin; an edge record has `copies` aligned with `origins`.
  Both are written only when a copy is involved; older readers drop them and see the original.
- An edge reference record has `origins`, its two faces' origins in the order of `faces`, written
  only when either is known; older readers drop it and lose only the fallback it gives. Files
  saved before origins were kept have them completed on opening (below).
- Records carry stable IDs. Construction curves add `"construction": true` only when set (on a
  point: loads as a point, reported). Expressions are canonical text with parameters as `$<id>`
  (`Expression::to_stored_text`, `parse_stored`); region keys and topology names 32-digit hex;
  numbers exact f64. Imports are `import` records (source name, STEP text); unreadable ones load
  empty, reported. Their text is parsed through `step_cache.rs`, a process-wide cache of solids by
  the blake3 digest of the text, bounded at 256 MiB by `Solid::approximate_size` and dropping the
  least recently used, so a load, a journal replay and a recovery scan of one model parse each
  import once.
- Reported fallbacks: unreadable extent becomes 10 mm or 360°; an unreadable extrusion end, or
  one whose face or plane cannot be read, becomes 10 mm; an unreadable angle of two becomes 180°;
  unreadable blend edge left out; unreadable opened face left closed; sketch whose face cannot be
  read, or that lies on a feature that is not a body or datum plane, stays on its stored plane;
  revolve whose axis line is gone turns about its sketch's vertical axis; pattern whose direction
  or axis cannot be read runs along X or turns about Z, an unreadable second direction is left
  out, an unreadable count becomes 1 and spacing 10 mm.

## Version history

- Each save that changes the model keeps the state it replaces as a version in the file
  (`save_with` reads earlier ones from the session's file, so Save As carries them; unchanged
  models add none).
- Version: info chunk (time, last change, blake3 digest) plus data chunk compressed with the next
  newer version as zstd prefix; every eighth stored whole, bounding a damaged chunk's chain. Info
  belongs only to the data chunk right after it: data with damaged info is kept (unlisted) so older
  deltas decode; info with damaged data is dropped. Rebuilt versions are checked against their
  digest before being restored.
- `history` lists versions and whether each can be rebuilt, judged from the chain rather than by
  decoding (`Parsed::rebuildable`): a whole version in a known codec can; a delta can when the
  version it is stored against can (the head's records matching its digest for the newest), and
  when a damaged piece precedes it since the previous version's data (`StoredVersion::after_damage`,
  so it may be stored against a version that is gone) it is decoded from the nearest whole one and
  checked against its digest, or, having no info, against the next listed one's. An undamaged
  file's listing thus unpacks at most the head's records. A version whose data passes its checksum
  but does not rebuild to its digest is listed and refused on restore.
  `load_version` rebuilds from the nearest whole version at or after it. When the head cannot be
  rebuilt, the next save drops the deltas that depended on it.
- Retention (`retention.rs`), on each save adding a version: the ten newest listed stay; older keep
  the newest of each hour for a day, day for thirty days, week for a year, thirty days beyond
  (slots by absolute time); unlisted always stay. A delta whose newer neighbour was dropped is
  decoded (each version at most once per pass, only along runs that need it) and recompressed
  against the newest kept version before it, or stored whole when a dropped version was whole; one
  that cannot be decoded is copied as it was. A first pass decodes along the same runs, and when a
  delta could not be decoded for want of memory (`OverBudget`, `OutOfMemory`, or a newer neighbour
  that failed so) the versions dropped before it are kept, so thinning never leaves a delta whose
  newer neighbour is gone; should the second pass still run short, the save fails
  (`EncodeError::HistoryTooLarge`) rather than write such a delta.
- Size: a model file stays within the 2 GiB `MAX_FILE_SIZE` every reader enforces
  (`LARGEST_FILE` in `binary/model.rs`). When the versions would take the file past three
  quarters of it, the oldest kept versions are dropped (a tail, so nothing left depends on them)
  until it fits, and `Encoded::dropped_for_size` counts the listed ones; the save then says how
  many were removed. A model too large on its own is refused (`EncodeError::ModelTooLarge`), since
  the file could not be opened again.
- Versions copied unchanged are placed for block sharing: each run at least 256 KiB long lands at
  the same offset modulo 4 KiB as in the file being replaced (a zero-filled `Padding` chunk,
  skipped by readers, starts the versions for the first such run and precedes each later one);
  `copy_file_range` (`save.rs`) clones each run's whole blocks. What could not be cloned is written
  from memory, as is every cloned range when the file changed (size or times) after it was read.

## Saving

- Write a temporary sibling, fsync, rename over the target, fsync the directory. Permissions, group
  and extended attributes (ACLs too, via `xattr`; unsettable ones skipped) are kept; the temporary
  is owner-only until applied. A symbolic link is followed and its file replaced. A failed
  directory fsync after the rename is logged, not a failed save.
- Temporaries are `.<name>.<machine>-<boot>-<pid>-<n>.tmp`, `<machine>` the start of
  `/etc/machine-id` and `<boot>` the start of the kernel's `boot_id`. After each write, those next
  to the file are removed when they come from an earlier boot of this machine, or from this boot
  with their process gone; other machines' saves on a shared folder are left alone, and with no
  machine id only this boot's are swept. The scan of the recovery directory sweeps it the same way
  (untitled journals' temporaries). Temporaries without a machine (`<boot>-<pid>-<n>`) are swept
  by boot and process alone.
- Names past 255 bytes (temporaries, `.damaged` copies) are cut and end in a hash
  (`paths::fitting`); a model whose name leaves no room for `.<name>.journal` keeps its journal in
  the recovery directory.
- A save given `SaveOptions::unless_changed_from` (the head digest the session loaded or last
  saved, `FileDigest`, from `Loaded::digest` or `Saved::digest`) refuses with
  `SaveError::ChangedOnDisk` when the file it replaces, being the one its versions are read from,
  now has another head (or none), leaving the file as it was. Replacing anyway makes the outside
  head the newest version, as any save does.
- A save refuses when the earlier versions in the file it replaces cannot be read, and when the
  file is read-only (no write bit, or `access` denies writing), since `rename` would replace it
  anyway; the failure notice offers Save As.
- Before the rename, a model save reads the synced temporary back through the same handle and
  checks that it parses undamaged, that its head digest is the one encoded, and that every record
  hashes to it (`binary::reads_back`); otherwise the temporary is removed and the file left as it
  was. This guards against the compressor and the shared-range copies.
- Overwriting a file that loaded with problems first keeps the original as
  `<name>.damaged.caditor` (`keep_copy`, shared with preferences): a hard link, or where links are
  unsupported a copy synced under a temporary name and moved into place whole. The same copy is
  kept when the file being replaced is the one the versions are read from and has gone bad since
  it was loaded (`Encoded::previous_damaged`: a damaged chunk, records not matching the head's
  digest, or something that is no longer a model), since the save drops what it cannot read.

## Reading (`read.rs`)

- Every file caditor reads (models, journals, imports, preferences, recent files) goes through
  `read.rs`: regular files only, at most `MAX_FILE_SIZE` (2 GiB), reserved fallibly; devices,
  pipes and huge sparse files are refused in words. Saving over a non-regular file is refused.

## Loading is partial

- Each record and sketch item is read on its own (`Lenient`) and assembled through
  `Document::apply`, so a loaded model satisfies the document invariants.
- Reported repairs: damaged or unknown (newer) records left out; a lost parameter still used
  becomes a stand-in of value 0; unusable or duplicate names renamed; a parameter cycle broken at
  the parameter closing it; an unreadable dimension takes its drawn length; a rollback bar above a
  feature that is not there goes to the end. Suppressed IDs of features that are not there are
  dropped silently (their loss is reported with the feature).
- A feature referring to a feature that is not in the file loads as saved, since deleting a
  feature may keep its dependents (`document.md`); it fails when computed, saying what it lost.
- No damaged chunk but records not matching the head's digest (cut between chunks): reported as
  ending early; loads with problems and keeps its `.damaged` copy on the next save.
- `load` and `load_version` (not `decode`, which fuzzing and recovery scans use) then complete
  the origins of face and edge references saved without them (`complete_origins`,
  `document.md`), against the model exactly as saved, within `ORIGIN_COMPLETION_TIME` (20 s);
  the completed model is the session's saved baseline, so it opens unmodified and the next save
  writes the origins. A journal snapshot is not completed.
- Near-linear on hostile files: names indexed; duplicate IDs and cycles (`DependencyGraph`) found
  before applying; each kind of record applied as one transaction, halved only where it fails; at
  most `MAX_RECORDS` (10 000) parameters and features loaded, the rest reported.
