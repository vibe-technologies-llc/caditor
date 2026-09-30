---
paths:
  - "crates/caditor-file/**"
---

# File format and persistence

- Binary, zstd-compressed, xxh3-checked; no text model format. Format number restarted at 1; every
  shipped version stays readable. Version 2 added `linear_pattern` and `circular_pattern`
  feature records, which version 1 readers leave out as a kind they do not know.
- Version 3 adds two feature records, each written only when needed: `extrude_to`, an extrusion
  with a side that is not a distance (`extent` is `{"one_side": {"end", "reversed"}}` or
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
- Chunks hold at most 256 MiB; longer content is sliced that size, each slice its own frame against
  the same prefix: all but the last `CONTINUED`, all but the first `CONTINUATION`. The parser joins
  a run of one kind into one logical chunk of at most 2 GiB (`Chunk::unpack`); a run broken by
  damage or another kind, and a continuation with no run, is one damaged piece (a resync mid-run
  never yields partial content). Older readers see only the first slice, which fails its digest.
- Records decode one at a time; one load, listing or restore decompresses at most 2 GiB.

## Values (`binary/value.rs`)

- Self-describing serde encoding: tagged null, booleans, LEB128 unsigned and negative integers, LE
  f64, strings, bytes, sequences and maps closed by an end tag; nesting limited. Enums like JSON
  (unit variant is its name, others a one-entry map).
- `Lenient` reads any record through a `serde_json::Value`: unknown fields ignored, unknown record
  kinds reported as from a newer version.
- A newer version adding something whose loss changes the model's meaning needs a new record kind
  or a must-understand chunk; older readers drop unknown fields of records they change.

## Model file

- Head chunk (save time, last change's name, blake3 digest of records), one zstd chunk per record,
  version history. Records: parameters, features, ID counters, `principal` (hidden principal
  planes, axes, origin; only when one is hidden; the journal snapshot carries it as a field).
- A record with unchanged understood content is written back exactly as stored (newer fields kept,
  not recompressed). Unknown chunk kinds are carried unless must-understand; loading reports those
  as left out, so the original is kept as `.damaged`.
- Records carry stable IDs. Construction curves add `"construction": true` only when set (on a
  point: loads as a point, reported). Expressions are canonical text with parameters as `$<id>`
  (`Expression::to_stored_text`, `parse_stored`); region keys and topology names 32-digit hex;
  numbers exact f64. Imports are `import` records (source name, STEP text); unreadable ones load
  empty, reported.
- Reported fallbacks: unreadable extent becomes 10 mm or 360°; an unreadable extrusion end, or
  one whose face or plane cannot be read, becomes 10 mm; an unreadable angle of two becomes 180°;
  unreadable blend edge left out; unreadable opened face left closed; sketch whose face or datum
  plane cannot be restored stays on its stored plane; revolve whose axis line is gone turns about
  its sketch's vertical axis; pattern whose direction or axis cannot be read runs along X or
  turns about Z, an unreadable second direction is left out, an unreadable count becomes 1 and
  spacing 10 mm.

## Version history

- Each save that changes the model keeps the state it replaces as a version in the file
  (`save_with` reads earlier ones from the session's file, so Save As carries them; unchanged
  models add none).
- Version: info chunk (time, last change, blake3 digest) plus data chunk compressed with the next
  newer version as zstd prefix; every eighth stored whole, bounding a damaged chunk's chain. Info
  belongs only to the data chunk right after it: data with damaged info is kept (unlisted) so older
  deltas decode; info with damaged data is dropped. Rebuilt versions are checked against their
  digest before being offered.
- `history` lists versions (whether each can be rebuilt), keeping only the rolling newer snapshot;
  `load_version` rebuilds from the nearest whole version at or after it. When the head cannot be
  rebuilt, the next save drops the deltas that depended on it.
- Retention (`retention.rs`), on each save adding a version: the ten newest listed stay; older keep
  the newest of each hour for a day, day for thirty days, week for a year, thirty days beyond
  (slots by absolute time); unlisted always stay. A delta whose newer neighbour was dropped is
  decoded (each version at most once per save, only along runs that need it) and recompressed
  against the newest kept version before it, or stored whole when a dropped version was whole; one
  that cannot be decoded is copied as it was.
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
- Temporaries are `.<name>.<boot>-<pid>-<n>.tmp`, `<boot>` the start of the kernel's `boot_id`;
  those of this boot whose process is gone are removed after each write (other machines' saves on
  a shared folder are left alone).
- Names past 255 bytes (temporaries, `.damaged` copies) are cut and end in a hash
  (`paths::fitting`); a model whose name leaves no room for `.<name>.journal` keeps its journal in
  the recovery directory.
- A save refuses when the earlier versions in the file it replaces cannot be read.
- Overwriting a file that loaded with problems first keeps the original as
  `<name>.damaged.caditor` (`keep_copy`, shared with preferences): a hard link, or where links are
  unsupported a copy synced under a temporary name and moved into place whole.

## Reading (`read.rs`)

- Every file caditor reads (models, journals, imports, preferences, recent files) goes through
  `read.rs`: regular files only, at most `MAX_FILE_SIZE` (2 GiB), reserved fallibly; devices,
  pipes and huge sparse files are refused in words. Saving over a non-regular file is refused.

## Loading is partial

- Each record and sketch item is read on its own (`Lenient`) and assembled through
  `Document::apply`, so a loaded model satisfies the document invariants.
- Reported repairs: damaged or unknown (newer) records left out; a lost parameter still used
  becomes a stand-in of value 0; unusable or duplicate names renamed; a parameter cycle broken at
  the parameter closing it; an unreadable dimension takes its drawn length.
- No damaged chunk but records not matching the head's digest (cut between chunks): reported as
  ending early; loads with problems and keeps its `.damaged` copy on the next save.
- Near-linear on hostile files: names indexed; duplicate IDs and cycles (`DependencyGraph`) found
  before applying; each kind of record applied as one transaction, halved only where it fails; at
  most `MAX_RECORDS` (10 000) parameters and features loaded, the rest reported.
