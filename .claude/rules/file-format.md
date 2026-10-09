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
- The model properties are a `properties` record, written only when one is set, each field only
  when not empty. Losing them changes nothing computed, but an older reader reports the unknown
  record and keeps the `.damaged` copy rather than dropping them silently. Loading normalises them
  as the edit does and cuts a field past its limit, reported. The journal snapshot carries the same
  record as `properties`, and `set_model_properties` holds it.
- The saved views are a `views` record, written only when a view is saved or the Isometric view is
  redefined: `named` (each `name` and a `view` of `target`, `orientation` as x, y, z, w and
  `distance`) and `home`, a view, each only when present. Losing them changes nothing computed, so
  like the properties an older reader reports the unknown record and keeps the `.damaged` copy
  rather than dropping them silently. Loading reads each view on its own: an unreadable or
  unusable one is left out, a blank name becomes "View N", a long one is cut, a repeated one is
  numbered and views past `MAX_SAVED_VIEWS` are left out, each reported (`restore_views`). The
  journal snapshot carries the same record as `views`, and `set_saved_views` holds it.
- The selection sets are a `selection_sets` record, written only when there is one: `sets`, each a
  `name` and `members`, each `body` (an id), `face` (`body` and a face record) or `edge` (`body`
  and an edge record). Losing them changes nothing computed, so like the views an older reader
  reports the unknown record. Loading (`selection_sets.rs`, `restore_selection_sets`) reads each
  set and member on its own: an unreadable member is left out, a set left with nothing is left
  out, names are repaired like the views' ("Set N", cut, numbered), members past
  `MAX_SET_MEMBERS` and sets past `MAX_SELECTION_SETS` are left out, each reported. The journal
  snapshot carries the same record as `selection_sets`, and `set_selection_sets` holds it.
- A feature record carries `group`, its folder's name, only when it has one; losing it changes
  nothing computed, so it is a field, and an older reader drops it. A name too long for this
  version is cut at `MAX_GROUP_NAME_CHARS`, reported. The journal's `set_feature_group` holds the
  same name, absent for none.
- A parameter record's `note` is written only when not empty, like a feature's `hidden`; an older
  reader drops it. A note too long for this version is cut at `MAX_PARAMETER_NOTE_CHARS`,
  reported.
- Which dimension or feature value a model parameter names is a `named_values` record, written
  only when some parameter has an owner: `values`, each the `parameter`'s id and its `owner`
  (`feature` with `feature` and `value`, or `dimension` with `sketch` and `constraint`). The
  parameter itself stays an ordinary `parameter` record, so an older reader keeps every value and
  every expression using it, lists it with the other parameters and reports the unknown record,
  keeping the `.damaged` copy. Loading applies the owners after the features: an unreadable entry
  is reported and its parameter listed with the others, a description past
  `MAX_VALUE_LABEL_CHARS` is cut, reported, and an entry for an absent parameter is dropped
  silently. The journal snapshot carries the same record as `named_values`, the journal's
  `insert_parameter` carries `owner` when there is one, and `set_parameter_owner` is its own
  record.
- Records carry stable IDs. Expressions are canonical text with parameters as `$<id>`
  (`to_stored_text`, `parse_stored`); region keys and topology names are 32-digit hex; numbers
  exact f64.
- An import is an `import` record (source name, STEP text, and the absolute `path` it was read
  from when known and valid UTF-8, written only then, so older readers drop it and lose only
  Reload); one that cannot be read loads empty, reported. An import placed anywhere but the origin
  is a `placed_import` record (the same fields plus `offset` and `turn`, three stored texts each;
  unreadable ones load as 0 mm or 0 deg, reported), since an older reader would put it at the
  origin; one at the origin is still written as `import`. A model or journal snapshot
  (`feature_records`) keeps each STEP text once: a `placed_import` whose text an earlier import in
  tree order already holds writes `shares`, that text's blake3 digest in hex, instead of `step`,
  and loading (`ImportTexts`) hands every import of one text the same text and solid; a shared
  text that cannot be found loads the import empty, reported. An `import` record always holds its
  text, for older readers, and journaled edits hold theirs whole. Parsing goes through `step_cache.rs`, a process-wide LRU cache of solids by the blake3
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
- A version's info record carries `kept: true` only for a version the user keeps (absent means not
  kept, so every older file reads with nothing kept). Losing it changes nothing computed, so it is
  a field, not a record kind: an older caditor ignores it, lists the version normally and writes
  the info chunk back as stored, so the flag survives its saves, but its own thinning knows no
  `kept` and may drop the version; the flag then goes with it.
- Marking is a file-level change, not a document one (`binary::with_version_kept`,
  `set_version_kept`, `Storage::keep_version`): versions belong to the file and not to the model, so
  it neither dirties the model nor enters the undo history, unlike restoring, which changes the
  model. It rewrites only the one info chunk (as a JSON-like map, so fields this version does not
  know survive) and copies every other chunk as stored, then goes through the same atomic
  replace and read-back as a save. It runs on the storage worker, in order with saves, since two
  writers each rewriting the file from what they read would lose one another's change; it is
  refused with `KeepVersionError::ChangedOnDisk` when the head is no longer the one the session
  loaded or saved, and for a damaged file, a file from a newer version or one holding a
  must-understand chunk (a save keeps a `.damaged` copy for those, a metadata flip must not).
- Retention (`binary/retention.rs`), on each save adding a version: the newest stay, older thin by
  age tiers; unlisted and kept always stay, and a kept version holds its tier slot like any other
  version that is kept, so older ones in it still thin. Only the size limit below can still drop a
  kept version, the oldest first. A delta whose newer neighbour was dropped is recompressed
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
  A slot adds `slot` (`length`, `angle` texts; 10 mm and 0 deg when unreadable) and a standard
  size `standard` (`size` as `M3`, `fit` as `close`, `normal`, `loose`, `tapped`, `tapped_fine`,
  `tapped_fine_2`, `tapped_fine_3` or `heat_set_insert`); a standard
  this version does not know loads as none, keeping the typed sizes, reported; so do the fits added
  later (`tapped_fine_2`, `tapped_fine_3`, `heat_set_insert`) in older readers, and a fit its size
  does not offer. A tapped thread other than the default carries `thread` (`class` when not the
  first, `left_handed` when set, `depth` text when not the whole wall); losing it changes no solid,
  only the cosmetic thread, so it is a field an older reader drops. An unknown class loads as the
  first, reported, and an unreadable depth as 10 mm. A hole sized by its circles is a `hole_by_circles` record of the same fields,
  since an older reader would drill the typed diameter at every circle, and one also scaling its
  counterbore or countersink by them a `hole_scaled_by_circles` record. A stepped hole is a
  `stepped_hole` record: `feature`, the hole record of its sizing whose style is a counterbore of
  the first step, and `steps` (each `diameter` and `depth` text; unreadable ones load as 10 mm and
  3 mm, reported), since an older reader would drill a single counterbore; an inner record that is
  no hole loads without the steps, reported. A hole ending in a drill point is a `drill_point_hole`
  record: `feature`, the hole record it would be ending flat (itself a `stepped_hole` when stepped),
  and `angle` (stored text; unreadable: 118 deg, reported), since an older reader would drill a
  flat bottom; an inner record that is no hole loads without it, reported.
- A datum point is a `point` record (`base`, a point reference: `origin`, `datum`, `vertex` with
  `body` and the vertex name's digest, `centre` with `body` and an edge record, `surface_centre`
  with `body` and a face record, or `sketch` with `sketch` and `entity`; `offset`, three stored
  texts). Planes through references are
  `plane_through` records (`points`, `midway`, `axis_and_point`, `normal_to`) and the new axis forms
  `axis_through` records (`points`, `normal_to`), so an older reader reports them rather than
  misreading them; an unreadable reference loads as the XY plane, the Z axis or the origin,
  reported. An axis reference may be a `sketch_line` (`sketch`, `entity`).
- A coordinate system is a `coordinate_system` record (`origin`, a point reference; `x_axis`, an
  axis reference; `plane`, a plane reference; `reverse_x` and `reverse_z` only when set), its own
  kind so an older reader reports it; an unreadable reference loads as the origin, the X axis or
  the XY plane, reported. An axis reference to one of its axes is `frame` (`frame`, its id, and
  `axis`), a plane reference to one of its planes `frame` (`frame` and `plane`). A sketch lying on
  one of its planes is a `sketch_on_frame` record (`feature`, the sketch record, and `frame`, the
  same `frame` and `plane`), and a move in it a `move_in_frame` record (`feature`, the move record
  it would be in the world, itself possibly turning about its centre or an axis, and `frame`), since
  an older reader would leave the sketch where it was or move along the world's axes; an inner
  record of another kind loads without it, reported. The journal's `set_sketch_placement` carries
  `frame` for a sketch placed on one. A scale whose centre is measured in one is a
  `scale_in_frame` record and an import placed in one an `import_in_frame` record, each the same
  `feature` and `frame` (the inner import record is `import` or `placed_import` as its offsets
  and turns decide, `BodyPlacement::is_unmoved`), since an older reader would scale about the
  world's point or place the import in the world. A point reference to a system's origin is
  `frame` (its id); a datum using one is a `frame_origin_datum` record wrapping the datum's
  record (`feature`), so an older reader reports the datum as from a newer version rather than
  damaged. An import scaled as it is read is a `scaled_import` record (`feature`, the import
  record as above, and `scale`, its stored text; unreadable: 1, reported), inside any
  `import_in_frame`, since an older reader would keep the file's size.
- The constructed planes are `plane_construction` records (`tangent` with `body`, `face` and
  `toward`, a point reference; `square_to_curve` with `body`, an `edge` record and the `distance`
  text; `lines`, two axis references) and the constructed points `point_construction` records
  (`lines_cross`, two axis references; `axis_and_plane`; `three_planes`; `along`, a station like
  `square_to_curve`'s), each a kind of its own so an older reader reports it. An unreadable
  reference loads as the XY plane or a point at the origin, reported, and an unreadable distance
  as 0 mm. The datums at curved faces, edge middles and face centres are `datum_construction`
  records (`tangent_at` and `square_to_face`, each `body`, `face` and `toward`; `edge_middle` with
  `body` and an `edge` record; `face_centre` with `body` and a `face` record), a kind of its own
  added later so an older reader reports it rather than calling the feature damaged; an unreadable
  reference loads as the XY plane, the Z axis or a point at the origin, reported.
- A `move` feature record holds `body` and the stored text of its three distances (`offset`) and
  three turns (`turn`); an unreadable one loads as 0 mm or 0 deg, reported. A copying move is a
  `copy` record of the same fields, since an older reader taking it for a move would move the
  original. A move or copy turning about its body's centre is a `move_about_centre` record whose
  `feature` is that record, since an older reader would turn it about the origin; an inner
  record that is no move loads without it, reported. One turning about an axis is a
  `move_about_axis` record holding the same inner `feature`, its `axis` (an axis reference) and the
  stored text of its `angle` (unreadable: 0 deg); an axis that cannot be read loads turning about
  the body's centre, reported.
- A `mate` feature record holds `body`, `pair` (`faces` with `face`, a face record, `target`, a
  plane reference, and the `distance` text; or `axes` with `axis` and `target`, axis references)
  and `flipped` only when set. An unreadable moving face loads as a reference to no face, so the
  mate fails until it is chosen again; an unreadable target plane as the XY plane, an axis as the
  Z axis and a distance as 0 mm, each reported.
- A `mirror` feature record holds `body`, `plane` (a plane reference; an unreadable one loads as
  the YZ plane, reported) and `keep_original`. One mirroring features rather than its whole body
  is a `feature_mirror` record: `feature`, the `mirror` record it would be mirroring the body, and
  `mirrored`, the ids of the features it reflects in tree order, since an older reader would
  mirror the whole body; an inner record that is no mirror loads without them, reported, and a
  repeated id is kept once. A `scale` record holds `body`, the stored text of
  `factor` (unreadable: 1) and of the three `center` lengths (unreadable: 0 mm).
- A feature record carries `appearance` only when a body has one: `colour` as `#rrggbb`,
  `material`, `density` as stored text, the body's own `name`, `opacity` (a percent) and `faces`
  (each a face record with its `colour` and an optional `opacity`), each optional; an unreadable
  face colour is left out, reported, and a face opacity outside `MIN_OPACITY_PERCENT` to 100 is
  dropped, reported (a reader older than face opacities draws the face like its body). Losing it changes nothing computed, so
  it is a field, not a record kind; an unreadable part is left out and reported, the rest kept.
  The journal's `set_body_appearance` holds the same record. `describe_unreadable_record` skips
  the feature fields (`FEATURE_FIELDS`) when naming an unknown kind.
- A pattern with a total length or instances left out is a `pattern` record (`shape`: `linear` or
  `circular`, each the older record's fields, a direction's `total: true` when measured overall;
  `skipped`, a list of `[step, step]`), since an older reader would make the wrong copies; any
  other pattern is still written as `linear_pattern` or `circular_pattern`. Loading drops skipped
  entries that no pattern can make (the original, a step past `MAX_PATTERN_INSTANCES`).
- A pattern repeating features rather than its whole body is a `feature_pattern` record:
  `feature`, the pattern record it would be repeating the body, and `repeated`, the ids of the
  features it repeats in tree order, since an older reader would repeat the whole body; an inner
  record that is no pattern loads without them, reported, and a repeated id is kept once.
- An extrusion or revolve cutting other bodies too is a `cut_several` record: `feature`, the
  record it would be without them, and `bodies`, the ids of the others, so an older reader
  reports it rather than cutting one body; an inner record that is no extrusion or revolve loads
  without the bodies, reported.
- A revolve keeping one side of its axis is a `revolve_one_side` record: `feature`, the revolve
  record it would be turning the whole profile, and `side` (`left`, `right`), since an older reader
  would fail on the crossing profile or turn both sides; an inner record that is no revolve loads
  turning the whole profile, reported. `cut_several` wraps it when the revolve also cuts other
  bodies.
- A tapered or thin-walled extrusion, or a thin-walled revolve, is a `shaped_sweep` record:
  `feature`, the record it would be without them (which `cut_several` or `revolve_one_side` may
  wrap), `taper` (stored text, only when set; unreadable: 0 deg, reported) and `wall` (`thickness`
  as stored text, unreadable: 1 mm, reported, and `side`: `inside`, `outside`, `centred`; an
  unreadable wall loads as 1 mm centred, reported), since an older reader would sweep the plain
  profile; an inner record that is no extrusion or revolve loads without them, reported, as does
  a revolve's taper.
- A `split` feature record holds `body`, `plane` (a plane reference; an unreadable one loads as the
  YZ plane, reported) and `flipped` only when set. A split along another body or a sketch's curve
  is a `split_along` record (`body`, `along` as `{"body": id}` or `{"sketch": id}`, `flipped` only
  when set), since an older reader would not know what to split along; a split along a plane is
  still written as `split`.
- An `offset_face` feature record holds `body`, `distance` (stored text; unreadable: 1 mm,
  reported), `faces` (face records; an unreadable one is left out, reported as left where it is)
  and `tangent` only when set.
- A `primitive` feature record holds `shape` (`box` with `length`, `width`, `height`; `cylinder`
  with `diameter`, `height`; `sphere` with `diameter`; `torus` with `diameter`, `tube`; `cone` with
  `bottom`, `top`, `height`; `wedge` with `length`, `width`, `height`, `top`; `prism` with `sides`,
  `diameter`, `height`, each stored text; unreadable: 10 mm, a tube 2 mm, a cone's top or a
  wedge's top 0 mm, sides 6, reported), `plane` (a plane reference; unreadable: the XY
  plane, reported), `at` (two stored texts; unreadable: 0 mm, reported), `anchor` (`corner`,
  `base_centre`, `centre`), `operation` as an extrusion's, and `reversed` only when set.
- A `thread` feature record holds `body`, `face` (a face record; unreadable digests load as a
  reference to no face, so the thread fails until the face is chosen again, reported),
  `standard` (`metric_coarse`, `metric_fine`, `iso_228`, `iso_7`, `trapezoidal`; unknown loads as
  metric coarse), `size` (`M8`, `M8x1`, `1/2`, `Tr 20x4`; unknown loads as the size nearest 8 mm),
  `class` (absent for the one internal class of ISO 228; unknown loads as the standard's first
  internal class), `left_handed` and `reversed` only when set and `depth` (stored text; absent
  for the whole face, unreadable 10 mm), each fallback reported. A tapped hole's thread is not a
  `thread` record: it comes from the hole's `standard` and its `thread` field.
- A `combine` feature record holds `body`, `tool` and `operation` (`join`, `cut`, `intersect`).
  One with more tool bodies or keeping its tool is a `combine_tools` record: `feature`, the
  `combine` record it would be with the first tool alone, `more_tools` (ids, only when there are
  any) and `keep_tool` (only when set), since an older reader would consume one tool and miss the
  rest; an inner record that is no combine loads without them, reported, and a repeated tool is
  kept once. A `remove` record holds `body`.
- A diameter across an axis (`AxisDiameter`) is a `distance` record with `diameter: true` holding
  the radius: a literal halved, any other expression divided by 2 (loading takes the division
  off again or doubles), so an older reader drops the flag and holds the same geometry as a
  distance.
- An ellipse is an `ellipse` entity record (`center`, `major`, `minor_radius`) and an elliptical
  arc an `elliptical_arc` one (the same plus `start` and `end`); their radii are `major_radius` and
  `minor_radius` constraint records (`ellipse`, `value`), an unreadable value falling back to the
  drawn radius. Older readers report each as a kind from a newer version and keep the points.
  Naming an unknown kind skips the record's flags (`id`, `name`, `construction`, `inactive`,
  `label`), so a construction ellipse is named by its kind.
- A sketch's constraint record carries `inactive: true` only for a disabled constraint (absent
  means active, so older files read unchanged); the journal's `add_sketch_constraint` carries the
  same flag and `set_sketch_constraint_active` is its own record.
- A dimension placed by dragging its label carries `label`, its offset as two numbers, only when
  placed; losing it changes nothing computed, so it is a field an older reader drops, putting the
  label back in its usual place. An offset that cannot be set loads without it, reported. The
  journal's `add_sketch_constraint` carries it too and `set_sketch_label` is its own record.
- A sketch record carries `projections` only when it has projected geometry: each the projected
  entity's `entity` and its `source` (`edge` with `body` and an edge record, `vertex` with `body`
  and the vertex name's digest, `sketch_entity` with `sketch` and `entity`, `section` with `body`
  and an edge record, `datum_plane` with `datum` and a finite positive `reach`, or
  `principal_plane` with `plane` (`xy`, `xz`, `yz`) and a finite positive `reach`). An older
  reader lacking a source kind keeps its geometry as ordinary geometry, reported. The projected flags
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
- `load_cancellable` is `load` under a `CancelToken`, checked after reading the file, between
  records (`binary::decode_cancellable`) and during origin completion; a cancelled load is
  `LoadError::Cancelled`, never a partial model.
- Near-linear on hostile files: names indexed; duplicate IDs and cycles (`DependencyGraph`) found
  before applying; each kind of record applied as one transaction, halved only where it fails; at
  most `MAX_RECORDS` parameters and features loaded, the rest reported.

## Clipboard (`clipboard.rs`)

- What caditor puts on the system clipboard is text: a header line
  `caditor clipboard: <kind>, version <n>` (`ClipboardKind::SketchGeometry` or `Features`,
  `CLIPBOARD_VERSION`), then one line of JSON made of the model file's own records
  (`EntityRecord`, `ConstraintRecord`, `FeatureRecord`, each `Lenient`), so a damaged or unknown
  item is left out with a note and the rest pastes, as when loading.
- Each payload names its `source` (the copying process and document session, chosen by the app)
  and carries the parameters its expressions use by id, name and value
  (`CarriedParameters`, values as stored literal text); `PasteOrigin` is `ThisDocument` only when
  the source is the reader's own, and parameters are then kept by id (`document.md`).
- Sketch geometry is written from a scratch sketch the clip is pasted into and read back through
  `restore_sketch` and `Sketch::clip`, so the same checks as loading apply; dimensions whose
  parameters cannot be carried are left out with a note. The payload also names the sketch it was
  copied from, so pasting into it again can shift the copy.
- Reading refuses in words (`ClipboardError`): text without the header (`Foreign`), another kind,
  an unknown kind, a newer version, damaged JSON, nothing usable, and text over
  `MAX_CLIPBOARD_TEXT`. The header tolerates a byte-order mark and CRLF line ends.
