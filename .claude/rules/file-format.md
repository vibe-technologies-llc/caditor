---
paths:
  - "crates/caditor-file/**"
---

# File format and persistence

- Binary, zstd-compressed, xxh3-checked; no text model format. `FORMAT_VERSION` is 1, the first
  shipped format; every shipped version stays readable forever.
- Files written by development builds before version 1 start with `\x89CAD\r\n\x1a\n` and mark
  chunks `CDCK`: loading refuses them as `LoadError::FromEarlierBuild` ("made by an early
  development build of caditor") before any parsing, and since their chunks do not carry the
  current marker, one whose magic is damaged is never salvaged either (`NotAModel`).
- Evolving it:
  - Something whose loss changes what the model computes (suppression, the rollback bar) is a
    record kind of its own, not a feature field, so an older reader reports it as from a newer
    version instead of silently ignoring it.
  - A feature kind older readers lack gets its own record kind, but a feature the older kind can
    express is still written as it.
  - Optional derived data (`region_references`, `copies`, `origins`) is a field written only when
    known; older readers drop it and lose only the fallback it gives.
  - Something that is no part of the model and only decorates the file (the `Preview` chunk) is a
    chunk kind of its own, not must-understand: an older reader skips it without a word.
  - Anything else that must not be lost needs a must-understand chunk.
  - Every shipped version keeps files in the format corpus, which proves it still reads.

## Format corpus (`corpus/`, `tests/corpus.rs`)

- `crates/caditor-file/corpus/<format>-<version>/` holds files caditor wrote in each shipped
  version of each format it owns: `model-1/` (every record kind, feature record kind, sketch entity
  and constraint kind and entity flag, `pieces.caditor` holding trimmed and broken pieces with their
  `split_from`, a version history with a kept version and block-alignment padding, sliced
  content, a thread and a tapped hole carrying `modelled`, `reference-images.caditor` with a
  picture shared by two images, `preview.caditor` with a model's preview picture), `journal-1/` (every journaled edit, apply, undo and redo whole and
  by reference, a rebased snapshot, sliced content, `reference-images.journal` writing each
  picture once) and
  `parameters-1/` (an exported parameter file). Committed as binary (`.gitattributes` `-text`), a few KiB each.
- Write-once: a corpus file is never rewritten, regenerated or deleted. `KEPT` lists every file with
  the blake3 digest of a description of what it reads as (issues, records re-encoded by today's
  encoder, every version for models; header, base, entries and the replayed result for journals;
  the parsed rows for parameter files). On every run the suite checks the folder holds exactly the
  listed files, each reads to its recorded digest and loads without a problem, and on a mismatch
  prints the description. A digest changes only when today's encoder writes the same model
  differently, never because an old file reads differently, and the commit changing it says why.
- Adding to it: the writer
  `CADITOR_WRITE_FORMAT_CORPUS=1 cargo test -p caditor-file write_format_corpus -- --ignored --nocapture`
  writes the files of the current versions that do not exist yet (opened `create_new`, never over
  one) and prints their `KEPT` lines to paste. The suite fails until the corpus holds a file of
  every current `FORMAT_VERSION`, `JOURNAL_VERSION` and `PARAMETERS_VERSION`
  (`the_format_corpus_holds_files_of_every_current_version`), and until some file holds every
  variant of `Record`, `FeatureKindRecord`, `EntityKindRecord`, `ConstraintKindRecord`,
  `EditRecord`, `ChunkKind` and `Codec`, every entity flag (`ENTITY_FLAGS`), and a model and a
  journal hold content sliced over several chunks
  (`the_format_corpus_holds_every_kind_this_version_writes`, reading the variants from serde). So a
  version bump, a new kind or a new entity flag adds a builder to `corpus_of_this_version` under a
  new file name and runs the writer, in the same commit.
- The fuzz seed writer refuses any path inside the corpus (`seeds::seed_path`).

## Container (`binary/`)

- 8-byte magic (models `\x89CDM\r\n\x1a\n`, journals `\x89CDJ\r\n\x1a\n`), LE `u32` version,
  chunks. Chunk: marker `CDCH`, kind, codec
  (stored, zstd, or zstd against the next newer version as raw prefix), flags (`MUST_UNDERSTAND`,
  `CONTINUED`, `CONTINUATION`; unknown bits ignored), lengths, xxh3-64 of header and payload.
- A bad chunk makes the reader scan to the next marker with a valid checksum; hashed bytes are
  capped (`HASHING_ALLOWANCE`) so forged headers cannot make that quadratic; `parse_scanned`
  reports the bytes hashed and skipped for the test that holds it to that cap.
- A model with damaged magic or version is salvaged (`binary::salvage`) when a head chunk passing
  its checksum starts within `SALVAGE_REACH`: it loads with the version unknown, reported as a
  damaged start, and keeps its `.damaged` copy on saving. Anything else, a journal included, is
  `LoadError::NotAModel`.
- Content over `MAX_CONTENT` is sliced, each slice its own frame against the same prefix (all but
  the last `CONTINUED`, all but the first `CONTINUATION`); the parser joins a run up to
  `MAX_JOINED_CONTENT` (`Chunk::unpack`). A run broken by damage or another kind is one damaged
  piece, so a resync mid-run never yields partial content.
- Memory is bounded on hostile files: records decode one at a time (unpack, hash with
  `RecordDigest` checked against the head afterwards, parse); a load unpacks at most
  `MAX_DECOMPRESSED` in all (`Budget`); a version rebuild bounds what is alive per step
  (`unpack_beside`). `Budget::unpack` keeps the `UnpackError`: an unknown codec reads like content
  from a newer version, running out of memory or budget says too large to load, only the rest is
  "damaged".
- A save holds the model twice at most, uncompressed (`PriorRecords`, `NewRecords`).
- The `Preview` chunk (kind 14, `ChunkKind::Preview`) holds one PNG, at most `MAX_PREVIEW_BYTES`
  (512 KiB), written right after the head chunk so that `read_preview` (`preview.rs`) finds it in
  the first bytes of the file: it reads at most `PREVIEW_PREFIX`, walks magic, head chunk and the
  chunk after it, and answers the PNG only when that chunk passes its checksum, is a `Preview`
  and is a PNG (`Preview::from_png`, which refuses anything else or anything over the limit); no
  model, record or version is read, so Home's tiles cost one small read each. It is not a
  must-understand chunk, is never part of the record digest, and belongs to the file, not to the
  document: loading ignores it (`Parsed::preview`), a damaged one is an ordinary damaged piece, and
  a save writes the picture it was given or none, never the previous file's, so a preview never
  outlives the model it pictured. Marking a version kept (`with_version_kept`) carries it as stored.
  A reader older than the chunk skips it as an unknown non-must-understand chunk, and re-saving
  there carries it along after the records (`Parsed::foreign`), where `read_preview` no longer
  looks, so Home shows no picture for that file until it is saved here again.
- A preview comes from the caller (`PreviewSlot`, `PreviewDrawing`): the slot goes into
  `SaveOptions::preview` and `SaveRequest::preview`, the drawing half is filled by whoever draws the
  picture, on another thread. `encode_keeping` waits for the slot only after encoding the records,
  at most `PREVIEW_PATIENCE` (1.5 s); a drawing that is declined, dropped or too late leaves the
  chunk out and the save goes on, so a failed picture never fails or noticeably delays a save. A
  late picture is ignored (`PreviewDrawing::is_wanted` says so). The recovery journal carries no
  preview (`file-journal.md`).

## Values (`binary/value.rs`)

- Self-describing serde encoding with a nesting limit; enums like JSON.
- Only finite floats are encoded: NaN or infinity fails as `ValueError::NonFinite` and the save is
  refused with a reason, since the lenient reader would turn it into `Null` and load the record as
  damaged.
- `Lenient` reads a record through a `serde_json::Value`: unknown fields ignored, unknown record
  kinds reported as from a newer version.
- `ValueError::Refused` is what serde's `custom` becomes and carries no message; only serde's own
  derives raise it.

## Model file

- Head chunk (save time, last change's name, blake3 digest of records), the optional `Preview`
  chunk, one zstd chunk per record (`RECORD_KINDS`), version history. The journal snapshot carries `principal`, `suppressed` and
  `rollback` as fields.
- The `principal` record lists `hidden` geometry and, as a field written only when some is, `shown`
  geometry; an older reader drops `shown` and those show only when needed again. The journal writes
  a change to `Hidden` or `WhenNeeded` as the `SetPrincipalHidden` older readers know (`hidden` true
  or false) and only a change to `Shown` as `SetPrincipalShown`.
- A record with unchanged understood content is written back exactly as stored (newer fields kept).
  New records are encoded first; an old record whose bytes hash like a new one's is matched by
  them, any other by the digest of its re-encoded understood content (`understood_digest`), so only
  changed records are parsed. A match by understood content whose bytes differ re-encodes around
  the stored ones (`NewRecords::keeping_understood`). Unknown chunk kinds are carried unless
  must-understand; loading reports those as left out, so the original is kept as `.damaged`.
- Saving works records in batches of `PARALLEL_BATCH` (256) over threads (`each_in_parallel`,
  `parallel.rs`: the caller's interrupt carried, and whatever an interrupt stops worked on the
  calling thread, so nothing is ever left out): new records are encoded and hashed, the old file's
  unpacked and hashed (its unpacking budget still charged in order), and fresh records compressed,
  each batch appended in order, so the file is byte for byte the sequential one. A batch bounds
  what is held beside the snapshot; one batch of everything was barely faster and held another
  GiB on the VZ330, whose first save takes 5.3 s instead of 14 s and later ones 2.5 s instead
  of 3.9 s.
- Records carry stable IDs. Expressions are canonical text with parameters as `$<id>`
  (`to_stored_text`, `parse_stored`); region keys and topology names are 32-digit hex; numbers
  exact f64. Stored text reads as typed text does, with `$<id>` for names (`expression.md`); a
  change to the typed grammar that would read stored text differently needs a new format version.
- Unreadable values become reported fallbacks (the `restore_*` functions in `format.rs`), never
  refuse the file: each says what was set to what and keeps the feature.

### Optional document-level records

Each is written only when non-empty and is its own record kind; losing it changes nothing computed.
The journal snapshot carries the same record under the same name, held by the setter named.

| Record | Content and loading | Journal setter |
| --- | --- | --- |
| `properties` | Model properties, each field only when not empty; normalised as the edit does, over-limit fields cut. | `set_model_properties` |
| `views` | `named` (`name`, `view` of `target`, `orientation` as x, y, z, w, `distance`) and `home`. Each view read on its own (`restore_views`): unusable left out, blank name "View N", long cut, repeated numbered, past `MAX_SAVED_VIEWS` left out. | `set_saved_views` |
| `selection_sets` | `sets`, each `name` and `members` (`body` id; `face` or `edge` with `body` and a record). `selection_sets.rs`, `restore_selection_sets`: unreadable member or emptied set left out, names repaired ("Set N"), past `MAX_SET_MEMBERS` and `MAX_SELECTION_SETS` left out. | `set_selection_sets` |
| `configurations` | `values` (each `parameter`, `suppressed` or `colour` with its id), `rows` (`id`, `name`, `settings` each a `value` with `expression` stored text, `suppressed`, or `colour` as `#rrggbb`, absent for the default), `active`, `next_id`. The active row's values are also the model's own, so a reader without the record computes the same model. `configurations.rs`, `restore_configurations`: unreadable column or row left out, unreadable setting dropped, names repaired ("Configuration N"), repeated row id left out, unknown active leaves none; applied after suppression and the bar. | `set_configurations`, `set_active_configuration` |
| `named_values` | `values`, each the `parameter`'s id and its `owner` (`feature` with `feature` and `value`, or `dimension` with `sketch` and `constraint`). The parameter stays an ordinary `parameter` record. Applied after the features: unreadable entry reported and its parameter listed with the others, description cut at `MAX_VALUE_LABEL_CHARS`, entry for an absent parameter dropped silently. | `insert_parameter` carries `owner`; `set_parameter_owner` is its own record |

### Optional fields

- Feature `group` (folder name; cut at `MAX_GROUP_NAME_CHARS`; journal `set_feature_group`, absent
  for none). Parameter `note` (cut at `MAX_PARAMETER_NOTE_CHARS`). Feature `hidden`. Sketch
  constraint `inactive: true` (journal `add_sketch_constraint` carries it,
  `set_sketch_constraint_active` is its own record). Dimension `label` (offset as two numbers, only
  when placed; unsettable loads without it; journal `add_sketch_constraint` carries it,
  `set_sketch_label` is its own record). A `distance` record with `diameter: true` is an
  `AxisDiameter` holding the radius (a literal halved, other expressions divided by 2, undone on
  load).
- `appearance`, only when a body has one: `colour` as `#rrggbb`, `material`, `density` stored text,
  `name`, `folder` (the body's group path, an array of names, only when grouped; names past
  `MAX_GROUP_NAME_CHARS`, blank or unreadable cut the path there and levels past
  `MAX_BODY_FOLDER_DEPTH` are left out, reported with where the body is listed now; an older
  reader ignores it and lists the body ungrouped), `opacity` (percent), `finish` (`Finish::key`: `matte`, `satin`, `glossy`, `rubber`,
  `powder_coat`, `anodized`, `brushed_metal`, `polished_metal`) and `faces` (face record with
  `colour`, optional `opacity` and `finish`), each optional. Unreadable part, unknown finish, or a
  face opacity outside `MIN_OPACITY_PERCENT` to 100 is dropped, reported. Journal:
  `set_body_appearance`. Written as `BodyAppearance::settled` gives it, so a journaled request the
  document settles (blank material, opacity 100 or above) reads back. `describe_unreadable_record` skips `FEATURE_FIELDS` when naming an unknown
  kind.
- Sketch `projections`, only when projected: each `entity` and `source` (`edge` or `section` with
  `body` and an edge record, `vertex` with `body` and the vertex name's digest, `sketch_entity`
  with `sketch` and `entity`, `datum_plane` with `datum` and a finite positive `reach`,
  `principal_plane` with `plane` `xy`/`xz`/`yz` and `reach`, `datum_axis` with `datum` and
  `reach`, `principal_axis` with `axis` `x`/`y`/`z` and `reach`, `datum_point` with `datum`, or
  `origin`; the corpus holds the last four in `projected-datums.caditor`). The projected flags are
  derived on load after the constraints. Unknown or unreadable source (an older reader meeting a
  newer kind): geometry kept as ordinary, reported.
  Journal: `set_sketch_projection`, absent for none.

## Feature and entity records

Conventions for every record below:

- A wrapper record has `feature`, the record it would be without the extra, plus the extras; an
  inner record of the wrong kind loads without the extras, reported. A wrapper is a kind of its own
  so a reader not knowing it reports the feature as from a newer version instead of computing a
  different result; a case the inner kind can express is written as the inner kind.
- Stored texts are expression text; an unreadable one loads as the fallback given, reported. An
  unreadable reference (plane, axis, point, face) loads as the XY plane, the Z axis, the origin or
  a reference to no face (the feature then fails until chosen again), as noted, reported; a circular
  pattern's `axis` is optional (left out when none is chosen) and an unreadable one loads as none.
- `frame` references a coordinate system by id (`frame` plus `axis` or `plane`; a point reference
  to a system's origin is `frame` with its id).

Extrusions and revolves:

- `extrude`, `extrude_to`: `start` (stored text) only when it has one. A start at a face or plane
  and any revolve start are `extrude_from` and `revolve_from`: `start` is `distance` with its text
  or `plane` with a plane reference; unreadable loads as the sketch plane (or 0 mm).
- `shaped_sweep`: taper (`taper`, unreadable 0 deg) and thin wall (`wall`: `thickness`, unreadable
  1 mm, and `side` `inside`/`outside`/`centred`, unreadable 1 mm centred) for an extrusion or
  revolve; a revolve's taper is dropped. May wrap `cut_several` or `revolve_one_side`.
- `cut_several`: `bodies`, ids of the other bodies cut.
- `revolve_one_side`: `side` `left`/`right`; wrongly-kinded inner turns the whole profile.
- `extrude_along`: `direction`, an axis reference; unreadable runs square.
- `revolve_up_to`: `target` plane reference, `reversed` when set; unreadable is a full turn.
- `offset_ends`: `forward` (the one end of a one-sided extrusion, or the forward end) and
  `backward`, each an offset text only when set; unreadable 0 mm, offset for an end not running to
  a face left out. May be wrapped by `shaped_sweep`.
- `surface_ends`: an end up to a curved face; `forward` and `backward` (as `offset_ends`), each the
  face's attachment record only when that end runs up to one; unreadable face loads as 10 mm. The
  inner record may be an `offset_ends`.
- `sweep`: `sketch`, `regions` and `region_references` as an extrusion's, `path` (`unchosen`,
  `curves` with the path's `sketch` and its `curves` ids, or `edges` with `body` and edge records),
  `orientation` (`square`, `parallel`) and `operation`; other bodies cut wrap it in `cut_several`.
  An unreadable path loads as unchosen (the sweep fails asking for one) and an unreadable edge is
  left out, each reported. The corpus holds it in `path-sweeps.caditor`. `sweep_shape`: a wrapper
  around a sweep record (`feature`, itself possibly a `cut_several` or `along_helix`) with `twist`
  and `scale`, the twist's and end scale's texts (unreadable 0 deg and 1, reported), written only
  when either is not its literal default (`Sweep::is_shaped`); an older reader reports the sweep
  as from a newer version rather than sweeping it straight; a wrapped kind that is not a sweep
  loads without it, reported. The corpus holds it in `sweep-shapes.caditor`.
- `pipe`: `path` as a sweep's (unreadable loads as unchosen), `shape` (`round`, `square`,
  `triangle`), `size` (unreadable 10 mm), `wall` only when hollow (unreadable 1 mm), `bend`
  (unreadable 0 mm) and `operation`. The corpus holds it in `pipes.caditor`.
- `loft`: `profiles` (each `sketch`, and `region` with its key's digest and `reference` as a
  `region_references` entry when a region is chosen), `end` (point reference, only when set),
  `skin` (`smooth`, `ruled`) and `operation`. An unreadable profile is left out and an unreadable end
  loads ending at the last profile, each reported. The corpus holds it in `lofts.caditor`.
  `loft_ends`: a wrapper around a loft record (`feature`) with `closed` when set and `start` and
  `end`, the square ends' weight texts, only when set (unreadable 1, reported), written when the
  loft closes or has a square end (`Loft::has_ends_shaped`); an older reader reports the loft as
  from a newer version rather than leaving it open or free; a wrapped kind that is not a loft loads
  without it, reported. The corpus holds it in `loft-ends.caditor`.
- `coil`: `axis` (an axis reference; unreadable loads as the Z axis), `reversed` when set,
  `diameter` (unreadable 20 mm), `length` (`turns_and_pitch`, `height_and_turns` or
  `height_and_pitch` with its two texts; unreadable turns 5, pitch 5 mm, height 25 mm), `shape`
  (`round`, `square`), `size` (unreadable 2 mm), `placement` (`inside`, `centred`, `outside`),
  `taper` (unreadable 0 deg), `turning` (`counter_clockwise`, `clockwise`) and `operation`. The
  corpus holds it in `coils.caditor`.
- `helix`: `axis`, `reversed`, `diameter`, `length`, `taper` and `turning` as a coil's (same
  fallbacks). `along_helix`: a wrapper around a sweep or pipe record whose path is unchosen
  (`feature`) and `helix`, the helix feature's id, written for a sweep or pipe following a helix
  (inside `cut_several` for a sweep cutting other bodies); an older reader reports the feature as
  from a newer version rather than leaving it with no path. A wrapped kind that follows no path is
  loaded without it, reported. The corpus holds both in `helices.caditor`. `helix_start`: a wrapper
  around a `helix` record (`feature`) with `start`, the start angle's text (unreadable 0 deg,
  reported), written only when the start is not a literal zero; an older reader reports the helix
  as from a newer version rather than starting it elsewhere; a wrapped kind that is not a helix
  loads without it, reported. The corpus holds it in `helix-starts.caditor`. `helix_point`: a
  wrapper around a `helix` or `helix_start` record (`feature`) with `point`, the point reference
  the helix starts at (unreadable: the helix loads starting at its typed angle, reported), written
  only when `start_point` is set; an older reader reports the helix as from a newer version rather
  than starting it elsewhere; a wrapped kind that is not a helix loads without it, reported. The
  corpus holds it in `helix-points.caditor`. `flat_spiral`: the outermost wrapper around a
  `helix`, `helix_start` or `helix_point` record (`feature`), written only for a
  `HelixKind::FlatSpiral`, whose `length` height is the outer diameter; an older reader reports the
  spiral as from a newer version rather than winding a helix; a wrapped kind that is not a helix
  loads without it, reported. The corpus holds it in `flat-spirals.caditor`.
- `rib`: `sketch`, `body`, `thickness` (unreadable 2 mm, reported), `side` (`one_side`,
  `other_side`, `symmetric`), `direction` (`parallel`, `square`) and `reversed` only when set. The
  corpus holds it in `ribs.caditor`.
- `shaped_rib`: a wrapper around a `rib` record (`feature`) of a rib with a draft that is not a
  literal zero (`draft`, stored text; unreadable leaves 0°, reported) or free arc ends carried
  round their circles (`ends`: `round_arcs`), written only then, so an older reader reports the rib
  as from a newer version rather than building it without its draft; a plain rib reads as 0° and
  straight ends. A wrapped kind that is not a rib loads without them, reported. The corpus holds
  it in `shaped-ribs.caditor`.
- `separate_pieces`: a wrapper around the whole feature kind record (`feature`) of a feature whose
  `pieces` is a body each, written only then; an older reader reports the feature as from a newer
  version instead of making one body that later features' piece bodies would not find. Restoring
  unwraps it onto `Feature::pieces`; one nested inside another is unwrapped and reported. Bodies of
  pieces are stored as any body ID (a piece's ID is above `FIRST_UNSTORABLE_ID`, `document.md`), and
  the selection set and configuration readers accept them by their making feature's ID. The
  feature's `piece_looks` is an optional feature field (in `FEATURE_FIELDS`): each `piece` key with
  an `appearance` record (as `appearance`, settled) and `hidden` when set; an unreadable entry is
  left out, reported. `piece_stand_ins`, also an optional feature field: `curve` (a deleted key
  curve) and `carrier` (the curve standing in for it) per entry, an unreadable entry left out with
  one report and an entry carrying itself skipped; an older reader ignores it and keys such a piece
  by its lowest curve (corpus: `piece-stand-ins.caditor`, `journal-1/piece-stand-ins.journal`). Journal: `set_feature_pieces` (`one_body`, `body_each`), `set_piece_stand_ins`
  (`id`, `stand_ins` as above, whole set); a piece's look and
  hidden flag travel in `set_body_appearance` and `set_feature_hidden` under the piece's body ID. The
  corpus holds them in `separate-pieces.caditor` and `journal-1/separate-pieces.journal`.

Holes:

- `hole`: `sketch`, `body`, `diameter`, `depth` (`through_all`, or `blind` with text), `style`
  (`plain`, `counterbore` or `countersink` with texts), `reversed` when set; unreadable size loads
  as 5 mm, 10 mm, 3 mm, 90 deg. `slot`: `length`, `angle` (10 mm, 0 deg). `standard`: `size` (as
  `M3`) and `fit` (`close`, `normal`, `loose`, `tapped`, `tapped_fine`, `tapped_fine_2`,
  `tapped_fine_3`, `heat_set_insert`); an unknown standard, fit, or a fit its size lacks loads as
  none keeping the typed sizes. A non-default tapped
  thread carries `thread` (`class` when not the first, `left_handed` when set, `depth` text when
  not the whole wall, `modelled` when set; unknown class loads as the first, unreadable depth 10 mm,
  absent `modelled` is cosmetic): fields, so a reader without `modelled` draws the thread it would
  cut.
- Wrappers, each around the records before it where noted: `hole_by_circles` (same fields, sized by its circles),
  `hole_scaled_by_circles` (also scales counterbore or countersink), `stepped_hole` (`steps`, each
  `diameter` and `depth`; unreadable 10 mm and 3 mm; inner style is a counterbore of the first
  step), `drill_point_hole` (`angle`, unreadable 118 deg; may wrap `stepped_hole`), `hole_up_to` (may wrap any hole record; `end`: `up_to_next` or
  `up_to_face` with plane reference; `offset` only when set; unreadable end loads through all,
  unreadable offset 0 mm).

Edge treatments:

- `shaped_chamfer`: inner `chamfer` of the first distance, or `fillet` keeping the chamfer form it
  would switch back to, `form` (`two_distances` with `second`, or `distance_angle` with `angle`;
  unreadable 1 mm and 45 deg), `flipped` when set. An equal chamfer stays `chamfer`.
- `blend_groups`: inner `fillet`, `chamfer` or `shaped_chamfer` of its edges alone, `groups`, each
  `face` (face record), `body`, `convex`, `concave` or `made_by` (the feature's id); unreadable
  group left out, reported, so an older reader reads a rule as unreadable and drops it. The corpus
  holds the rules in `edge-rules.caditor`.

Datums:

- `point`: `base` (point reference: `origin`, `datum`, `vertex` with `body` and the name's digest,
  `centre` with `body` and an edge record, `surface_centre` with `body` and a face record, or
  `sketch` with `sketch` and `entity`) and `offset` (three texts). `plane_through` (`points`,
  `midway`, `axis_and_point`, `normal_to`), `axis_through` (`points`, `normal_to`). An axis
  reference may be `sketch_line` (`sketch`, `entity`).
- `plane_construction` (`tangent` with `body`, `face`, `toward`; `square_to_curve` with `body`, an
  `edge` record, `distance`; `lines`, two axis references), `point_construction` (`lines_cross`,
  `axis_and_plane`, `three_planes`, `along` with a station like `square_to_curve`'s),
  `datum_construction` (`tangent_at`, `square_to_face` with `body`, `face`, `toward`; `edge_middle`
  with `body` and an edge record; `face_centre` with `body` and a face record). Unreadable distance
  0 mm.
- `coordinate_system`: `origin` (point reference), `x_axis` (axis reference), `plane`,
  `reverse_x` and `reverse_z` only when set; unreadable loads as origin, X axis, XY plane.
  `frame_origin_datum` wraps a datum using a system's origin (`feature`).
- Frame wrappers: `sketch_on_frame` (inner sketch record, `frame`), `move_in_frame` (inner move
  record, `frame`), `scale_in_frame`, `import_in_frame` (inner `import` or `placed_import` as
  `BodyPlacement::is_unmoved` decides). Journal `set_sketch_placement` carries `frame`.

Moves, mates, mirrors, scale:

- `move`: `body`, `offset` (three texts), `turn` (three texts); unreadable 0 mm or 0 deg. `copy`:
  same fields, copying. `move_about_centre`: inner move or copy, turning about the body's centre.
  `move_about_axis`: same inner `feature`, `axis`, `angle` (0 deg); unreadable axis loads turning
  about the centre.
- `mate`: `body`, `pair` (`faces` with `face` record, `target` plane reference, `distance` text; or
  `axes` with `axis` and `target`), `flipped` when set. Unreadable face loads as no face, target as
  XY plane, axis as Z axis, distance 0 mm. Later kinds, each its own record: `face_axis_mate`,
  `angle_mate` (`sides`: `faces` or `axes`; `angle` text, unreadable 90 deg), `tangent_mate`
  (`sides` with round `face` and `target` plane, `flipped`), `face_on_round_mate` (`body`, flat
  `face`, `round_body`, `round` face record, `flipped`), `point_mate` (`body`, `point`, `target`:
  `point` or `plane`; unreadable point the origin).
- `mirror`: `body`, `plane` (unreadable YZ), `keep_original` (`AddToBody`, else `Replace`).
  `mirror_new_body`: `body`, `plane`, a mirror whose image is a body of its own (`NewBody`), a
  record kind of its own so an older reader reports it as from a newer version instead of joining
  the image to the body that later features' uses of the new body would not find; the inner record
  of the wrappers below when such a mirror holds features or faces. The corpus holds it in
  `mirrored-bodies.caditor`. `feature_mirror`: `mirrored`, ids of
  the features reflected in tree order (repeated id kept once). `face_mirror`: `faces` (unreadable
  left out; none left mirrors the whole body, reported), around a `feature_mirror` when a mirror
  holds both (the features win when computed, the faces are kept). `mirror_several`: inner mirror
  record, `bodies` (ids of the bodies mirrored besides its own, repeats and its own body left out),
  for a whole-body mirror of several bodies; the corpus holds it in
  `mirrored-several-bodies.caditor`.
- `scale`: `body`, `factor` (unreadable 1), three `center` lengths (unreadable 0 mm).
  `scaled_import`: inner import record, `scale` (unreadable 1), inside any `import_in_frame`.

Patterns:

- `pattern`: `shape` `linear` or `circular` (the fields of `linear_pattern` or `circular_pattern`, a direction's
  `total: true` when measured overall) and `skipped` (list of `[step, step]`); any other pattern is
  still `linear_pattern` or `circular_pattern`. Loading drops skipped entries no pattern can make
  (the original, steps past `MAX_PATTERN_INSTANCES`).
- `curve_pattern`: `body`, `sketch`, `count`, `spacing`, and `distance`, `follow`, `reversed`,
  `skipped` only when set. `point_pattern`: `body`, `sketch`, `base` point reference, `skipped`
  (the point's id plus one, in two words). Unreadable values 1 or 10 mm, base the origin.
- `feature_pattern`: `repeated`, ids of the features repeated in tree order (repeated id kept
  once).
- `pattern_new_bodies`: a wrapper around any pattern record (`feature`), written for a pattern whose
  `outcome` is `NewBodies` (inside `feature_pattern` when it also repeats features), so an older
  reader reports it as from a newer version instead of joining the copies that later features'
  copy bodies would not find; a wrapped kind that is not a pattern loads without it, reported. The
  corpus holds it in `patterned-bodies.caditor`.

Splits and faces:

- `split`: `body`, `plane` (unreadable YZ), `flipped` when set. `split_along`: `along` as
  `{"body": id}`, `{"sketch": id}` or `"unchosen"` (nothing chosen yet, waiting). `split_surface`: `body`, `face` attachment record; unreadable
  splits along YZ.
- `offset_face`: `body`, `distance` (unreadable 1 mm), `faces` (unreadable left in place),
  `tangent` when set. `delete_face`: `body`, `faces` (unreadable left on the body); the corpus holds
  it in `deleted-faces.caditor`. `move_face`: `body`, `faces` (unreadable left where they
  are), `distance` (unreadable 0 mm), `angle` (unreadable 0 deg), and `direction` and `axis` when
  chosen (axis references; unreadable left unchosen, reported); the corpus holds it in
  `moved-faces.caditor`. `emboss`: `body`, `faces` (unreadable left plain), `sketch` when chosen,
  `depth` (unreadable 1 mm), `recessed` when set, and `regions` (`{"chosen": [keys]}`, only when not
  every outline) with `region_references` as an extrusion's, a file without them reading as every
  outline; the corpus holds it in `embossed-faces.caditor` and `embossed-regions.caditor`. A
  drafted emboss (`Emboss::is_drafted`) is a `drafted_emboss` record of its own, `emboss` (the
  record above) and `draft` (unreadable 0 deg), so an older reader reports it rather than building
  straight walls; an undrafted one is still written as `emboss`. The corpus holds it in
  `drafted-emboss.caditor`.
  `draft`: `body`, `faces` (unreadable left as they stand), `neutral` (plane
  reference; unreadable loads as XY), `angle` (unreadable 0 deg), `flipped` and `tangent` when set,
  `beyond` (the angle beyond a parting line, only when split; unreadable 0 deg) and `own` (each a
  `face` record and its `angle`, only when any; unreadable entry left as it stands, angle 0 deg),
  each reported; the corpus holds it in `drafts.caditor`. `replace_face`: `body`, `faces`
  (unreadable left as they stand) and `target` when chosen (plane reference; unreadable loads with
  none, waiting for one), each reported; the corpus holds it in `replaced-faces.caditor`. `split_face`: `body`, `faces` (unreadable left whole), `along`
  (`{"plane": ref}` unreadable YZ, `{"body": id}`, `{"sketch": id}`, `"unchosen"`). Wrappers: `split_face_along`
  (`direction`, axis reference; unreadable carried square), `split_face_surface` (`face`
  attachment record; unreadable along YZ), `split_face_wrapped` (wraps round a cylinder or cone).

Bodies and solids:

- `primitive`: `shape` (`box`: `length`, `width`, `height`; `cylinder`: `diameter`, `height`;
  `sphere`: `diameter`; `torus`: `diameter`, `tube`; `cone`: `bottom`, `top`, `height`; `wedge`:
  `length`, `width`, `height`, `top`; `prism`: `sides`, `diameter`, `height`; unreadable 10 mm,
  tube 2 mm, cone or wedge top 0 mm, sides 6), `plane` (unreadable XY), `at` (two texts, 0 mm),
  `anchor` (`corner`, `base_centre`, `centre`), `operation` as an extrusion's, `reversed` when set.
- `thread`: `body`, `face` (unreadable digests load as no face), `standard` (`metric_coarse`,
  `metric_fine`, `iso_228`, `iso_7`, `trapezoidal`; unknown loads as metric coarse), `size` (`M8`,
  `M8x1`, `1/2`, `Tr 20x4`; unknown loads as the nearest 8 mm), `class` (absent for ISO 228's one
  internal class; unknown loads as the standard's first internal class), `left_handed`, `reversed`
  and `modelled` when set (absent is cosmetic), `depth` (absent for the whole face, unreadable
  10 mm). A tapped hole's
  thread is not a `thread` record (see hole).
- `combine`: `body`, `tool`, `operation` (`join`, `cut`, `intersect`). `combine_tools`: `more_tools`
  (ids, only when any, as written: a repeated tool fails the combine in words), `keep_tool` when set. `remove`: `body`.
- Imports: `import` (source name, STEP text, absolute `path` when known and valid UTF-8, written
  only then); unreadable loads empty, reported. Placed anywhere
  but the origin it is `placed_import` (same fields plus `offset` and `turn`, three texts each,
  unreadable 0 mm or 0 deg); at the origin still `import`.
- A record's `step` is a `StepField`: `Read` holds the text a file gave, `Kept` the model's
  compressed `StepText` (`document.md`), unpacked only while serialising, so building records never
  copies a text and a text that will not unpack fails the save instead of writing an empty import.
- A model or journal snapshot (`feature_records`) keeps each STEP text once: a `placed_import`
  whose text an earlier import in tree order holds writes `shares`, that text's blake3 digest in
  hex, instead of `step` (`WrittenTexts` unpacks and hashes only to write the digest or to compare
  with an earlier same-length text that is not the same `StepText`). Loading (`ImportTexts`) gives every import
  of one text the same text and solid; a missing shared text loads the import empty, reported. An
  `import` record always holds its text; journaled edits hold theirs whole.
- `step_cache.rs`: process-wide LRU of shared solids (`Arc<Solid>`, handed to `Import::shared`) by
  the blake3 digest of the text, so a load, journal replay and recovery scan parse each import
  once; its budget counts each solid once.
- Loading a model reads its import texts ahead, in parallel (`ImportTexts::reading_ahead`, the
  `in_parallel` of `parallel.rs` with the caller's interrupt): every distinct text an `import` or
  `placed_import` record holds, through the scaled, framed and separate-pieces wrappers the writer
  puts round them (`stored_step`), is read once by digest before features are restored, and
  `restore_import` takes that result (a text missed, as in a hand-nested record, is read where it
  is restored). Read one by one, the VZ330's 1,200 texts took 199 s to reopen; ahead, 75 s.
- `checked_texts.rs`: the blake3 digests of import texts whose `find_crossing` this computer
  already ran, so reopening a model reads them with `read_own_step` (`step_cache.rs`): an import
  marks each canonical text it stores (`canonical`, trusted exactly as the import trusts it), a
  load marks each text `read_step` accepted. A file from elsewhere is still checked once, and a
  crafted file cannot claim a text was checked. The app and the MCP server keep the digests in
  `checked-imports` in the state folder (`remember_checked_imports`; without it they live in
  memory only): magic `CDCHKD01`, 32-byte digests oldest first, an xxh3-64 of all that; a damaged
  file is forgotten whole, never partly trusted. At most `MAX_CHECKED` digests, the oldest leaving
  first; a load and an import each merge their new digests into the file under its update lock
  (`persist_checked`), so two sessions keep what each marked.
- `reference_image`: `picture` (the picture's blake3 digest in hex), `data` (its bytes as they were
  read, base64, since a record is read through `serde_json::Value`, which holds no byte strings;
  `reference_images.rs`, `PictureField`, which encodes a kept `Picture` only while serialising),
  `pixels` (width and height), `plane` (`{"plane": ref}` or `{"sketch": id}`), `at` (two texts),
  `turn`, `size` (`{"width": text}` or `{"calibrated": {"from", "to", "distance"}}` with two
  fractions per spot), `opacity`, and `flipped`, `in_front`, `locked` when set, written as
  `ReferenceImage::settle` leaves it, so a journaled request the document settles (an opacity below
  the least, a spot off the picture) reads back without a report. Unreadable plane
  loads as XY, size as 100 mm wide, values as 0 mm or 0 deg, spots held within the picture, an
  opacity below the least raised, each reported. A model or journal snapshot (`feature_records`)
  writes each picture's `data` once, in the first record of that digest in tree order
  (`share_picture`); later ones hold only the digest. Loading (`ImportTexts::pictures`, `Pictures`)
  gives every record of one digest the same `Arc<Picture>`; a picture whose data does not decode as
  base64, is not a picture caditor reads or has an impossible size, or a digest no earlier record
  carried, loads as `Picture::placeholder` (a grey square), reported, keeping the feature and its
  placement. The clipboard always writes `data`. An older reader not knowing the kind reports the
  feature as from a newer version. The corpus holds it in `reference-images.caditor`.
- The rendered view's environment picture (`environment.rs`, `read_environment_file`,
  `decode_environment`) is a Radiance HDR file only, found by content (`image::guess_format`),
  whose size is read from the header and refused past `MAX_ENVIRONMENT_PIXELS` before any pixel is
  decoded, then decoded under `Limits` to linear `f32` RGB (`EnvironmentPixels`). Failures are
  `EnvironmentReadError`s in words. `caditor-render` shapes and prefilters the pixels
  (`render.md`). OpenEXR is not read: `image`'s `exr` feature needs `pulp` and `paste`.
- Pictures are decoded only to be shown (`picture.rs`, `decode_picture`, never while loading):
  the format by `sniff_picture` (content, never the file name), the size read from the header
  first (`ImageReader::into_dimensions` without limits, JPEG XL's header) and refused past
  `MAX_PICTURE_SIDE` or `MAX_PICTURE_PIXELS` before any pixel is decoded, then decoded under limits
  (`image`'s `Limits`, `jxl-oxide`'s `AllocTracker`, `MAX_DECODER_BYTES`) to RGBA8 with the EXIF
  orientation applied: the `image` crate for PNG, JPEG (`zune-jpeg`), WebP, GIF (first frame), BMP,
  TIFF (first page, 8 and 16 bits), QOI and TGA, `jxl-oxide` for JPEG XL. Every failure is a
  `PictureReadError` in words (`Refused` with the `PictureError`, `Undecodable` naming the format,
  `UndecodableJpegXl`, `UnusualChannels`, `OutOfMemory`), never a panic. `read_picture_file` refuses
  a file past `MAX_PICTURE_BYTES` before reading it. Fixtures are made by the tests with `image`'s
  encoders and the `tiff` crate (a dev-dependency, for a two-page TIFF); JPEG XL and lossy WebP,
  which nothing in the workspace encodes, are the few-byte files in `pictures/`.
- `measurement`: `quantity` (`distance`, `angle`, `along`, `length`, `radius`, `area`, `sweep`,
  `perimeter`, `position`, `volume`, `mass`), `items` (one, two for `position`, three for `along`
  with the axis last; each `point`, `axis` or `plane` with a reference, `edge` with `body` and an
  edge record, `face` with `body` and a face record, `sketch` with `sketch` and `entity`, or `body`
  with `body`), `parameter` id only when it has one. Its parameter is an ordinary one holding the
  latest reading (`document.md`) and its owner an ordinary `named_values` entry. Unreadable or too
  few items load as the origin (X axis for an offset's or position's axis).

Sketch entities:

- `spline` (`control_points`) is an open control spline. Other kinds are their own records:
  `closed_spline`, `fit_spline` and `closed_fit_spline` (`fit_points`, evenly spaced),
  `centripetal_fit_spline` and `closed_centripetal_fit_spline` (`FitSpacing::Centripetal`), `conic`
  (`start`, `apex`, `end`, `rho`; an out-of-range rho is refused like any bad entity),
  `spline_piece` (`control_points`, `lead`, `trail`; what trim and extend leave of a control spline,
  refused with fewer than five points or end spans not above zero; `spline-pieces.caditor` in the
  corpus), `spline_knots` (`control_points`, `spans`; what adding a control point leaves, refused
  unless the spans are at least two, above zero, and three fewer than the points;
  `spline-knots.caditor`). A conic's rho dimension is a `rho` constraint (`conic`, `value`), falling back to the
  drawn rho.
- `ellipse` (`center`, `major`, `minor_radius`), `elliptical_arc` (plus `start`, `end`). Radii are
  `major_radius` and `minor_radius` constraints (`ellipse`, `value`, falling back to the drawn
  radius); `on_minor_axis` (`point`, `ellipse`). Naming an unknown kind skips the flags (`id`,
  `name`, `construction`, `split_from`, `inactive`, `label`).
- Constraints holding a point at a target: `horizontal_points` and `vertical_points` stay two point
  ids; held to any other target they are `horizontal_through` and `vertical_through` (`point`,
  `through`), on a line through a target `on_direction` (`point`, `through`, `line`, `square` when
  set), at the centre of corners `centroid` (`point`, `corners`). A target record is `point`,
  `middle`, `crossing` (two ids), `centroid` (ids), `quadrant` (`curve`, `side` of `right`, `top`,
  `left`, `bottom`) or `ellipse_end` (`ellipse`, `end` of `major`, `far_major`, `minor`,
  `far_minor`). A reader not knowing them reports each as a kind from a newer version and the point
  stays free; the corpus holds them in `point-targets.caditor`.
- An entity record has `construction` when true and `split_from` (the id of the curve it was split
  from, `sketch.md`) when set, both left out otherwise, in the model and in a journaled
  `add_sketch_entity`. A `split_from` the sketch refuses (the entity itself) is reported and
  dropped, the curve kept.
- A reader not knowing one reports it as a kind from a newer version and keeps the points.
- A sketch text (`sketch.md`, Text) is written among the entities as an entity record of kind
  `text` (`TextRecord`: `anchor`, `content`, `height` and `angle` stored texts, `bold` and `italic`
  only when set, `align` as `centre` or `right` only when not left, so a file from before
  alignment reads as left-aligned and one with line breaks is the same `content` string), so a reader not knowing it reports a kind from a newer version and keeps the
  anchor point rather than silently computing a model without the letters. Loading restores texts
  after the curves and before the constraints; an unreadable height loads as 5 mm and an angle as
  0°, reported, and a text the sketch refuses (missing anchor, blank or over-long content) is left
  out, reported with its content. Journal: `add_sketch_text` and `set_sketch_text` (`feature`,
  `id`, `text`) and `remove_sketch_text` (`feature`, `id`); an `add_sketch_entity` or
  `set_sketch_entity` holding a `text` kind is refused as unreadable. The corpus holds them in
  `texts.caditor`, `texts.journal` and, with line breaks and alignment, `text-layouts.caditor`.

- A stored gear (`sketch.md`, Gear) is an entity record of kind `gear` (`GearRecord`: `centre` and
  the stored texts `module`, `teeth`, `pressure_angle`, `profile_shift`, `root_fillet`, `bore` and
  `turn`), so a reader not knowing it reports a kind from a newer version and keeps the centre
  point. Loading restores gears after the texts and before the constraints; a value that cannot be
  read loads as 2 mm, 20 teeth, 20°, no shift, no fillet, no bore or no turn, reported, and a gear
  the sketch refuses is left out. Journal: `add_sketch_gear` and `set_sketch_gear` (`feature`, `id`,
  `gear`) and `remove_sketch_gear` (`feature`, `id`). Sketches written before it keep the free
  curves a gear was drawn as. The corpus holds them in `gears.caditor` and `gears.journal`.
- A meshing gear (`sketch.md`, Gear) is an entity record of kind `meshed_gear` of its own
  (`MeshedGearRecord`: `centre`, `driver` and the stored texts `teeth`, `bore` and `direction`),
  so a reader that knows plain gears but not pairs reports a kind from a newer version and keeps
  the centre point rather than reading a gear with values it never stored. Loading restores them
  after the plain gears, so a driver stands when its partner is read; an unreadable value loads as
  20 teeth, no bore or 0°, reported, and a meshing gear the sketch refuses (no such driver, a
  driver that itself meshes) is left out, reported. Journal: `add_sketch_meshed_gear` and
  `set_sketch_meshed_gear` (`feature`, `id`, `gear`), written for an `AddSketchGear` or
  `SetSketchGear` of a meshing gear and read back as those edits; removal is
  `remove_sketch_gear`. The corpus holds them in `gear-pairs.caditor` and `gear-pairs.journal`.
- A sprocket (`sketch.md`, Gear) is an entity record of kind `sprocket` of its own
  (`SprocketRecord`: `centre`, the `chain` designation as plain text and the stored texts `teeth`,
  `bore` and `turn`), so older readers report a kind from a newer version and keep the centre. A
  designation the table no longer knows leaves the sprocket out, reported naming it; an
  unreadable value loads as 17 teeth, no bore or 0°, reported. Loading restores sprockets after the
  meshing gears. Journal: `add_sketch_sprocket` and `set_sketch_sprocket` (`feature`, `id`,
  `gear`), written for an `AddSketchGear` or `SetSketchGear` of a sprocket and read back as those
  edits (an unknown designation makes the edit unreadable); removal is `remove_sketch_gear`. The
  corpus holds them in `sprockets.caditor` and `sprockets.journal`.

## Loading

- Every file caditor reads goes through `read.rs`: regular files only, at most `MAX_FILE_SIZE`;
  devices, pipes and huge sparse files are refused in words. Saving over a non-regular file is
  refused.
- Partial: each record and sketch item is read on its own (`Lenient`) and assembled through
  `Document::apply`, so a loaded model satisfies the document invariants.
- Reported repairs: damaged or unknown records left out; a lost parameter still used becomes a
  stand-in of value 0; unusable or duplicate names renamed; a parameter cycle broken at the
  parameter closing it; an unreadable dimension takes its drawn length; a rollback bar above an
  absent feature goes to the end. Suppressed IDs of absent features are dropped silently.
- A feature referring to an absent feature loads as saved (deleting a feature may keep its
  dependents, `document.md`); it fails when computed, saying what it lost.
- Features load in use order (`Document::in_use_order`, before applying): one stored above a
  feature it uses (by `features()` or a measurement it reads; only a damaged or hand-made file,
  since no edit leaves one) comes right after it, everything else keeping its stored order, and
  each such feature is reported ("“Base” was stored above “Outline”, which it uses, so it now comes
  after it."). Inserts refuse to land below a user (`BelowUser`), so without this the used feature
  would be left out. A file already in order costs one pass over the features. Features using each
  other in a circle are taken in stored order once nothing else can come, and the one that would
  land below a user is left out, reported.
- Records not matching the head's digest with no damaged chunk (cut between chunks) are reported as
  ending early; the model loads with problems and keeps its `.damaged` copy on the next save.
- `load_cancellable` and `load_version_cancellable` run under a `CancelToken`, checked after
  reading, between records (`binary::decode_cancellable`) and after rebuilding a version;
  cancelled is `LoadError::Cancelled`, never a partial model.
- Near-linear on hostile files: names indexed; duplicate IDs and cycles (`DependencyGraph`) found
  before applying; each kind of record applied as one transaction, halved only where it fails; at
  most `MAX_RECORDS` (`caditor-document`) parameters and features loaded, the rest reported. The
  `tally` counts the edits applied and the candidate names tried while loading.

## Version history

- Each save that changes the model keeps the state it replaces as a version in the file
  (`save_with` reads earlier ones from the session's file, so Save As carries them).
- Version: info chunk (time, last change, blake3 digest) plus data chunk compressed with the next
  newer version as zstd prefix; every `KEYFRAME_SPACING`th stored whole, bounding a damaged chunk's
  chain. Info belongs only to the data right after it: data with damaged info is kept (unlisted) so
  older deltas decode; info with damaged data is dropped. Rebuilt versions are checked against
  their digest before restoring.
- `history` judges rebuildability from the chain, not by decoding (`Parsed::rebuildable`). A
  version that passes its checksum but does not rebuild to its digest is listed and refused on
  restore. When the head cannot be rebuilt, the next save drops the deltas that depended on it.
- Info `kept: true` marks a version the user keeps (absent means not kept).
- Marking is a file-level change (`binary::with_version_kept`, `set_version_kept`,
  `Storage::keep_version`), not a document one: it neither dirties the model nor enters undo. It
  rewrites only the one info chunk (as a JSON-like map, so unknown fields survive), copies every
  other chunk as stored, then goes through the same atomic replace and read-back as a save. It runs
  on the storage worker in order with saves, and is refused with `KeepVersionError::ChangedOnDisk`
  when the head is not the one the session loaded or saved, and for a damaged file, a newer-version
  file or one holding a must-understand chunk (a save keeps a `.damaged` copy for those).
- Retention (`binary/retention.rs`), on each save adding a version: the newest stay, older thin by
  age tiers, both set by the save's `Retention` preset (`SaveOptions::retention`, carried by the
  storage worker's `SaveRequest`; `binary::encode_keeping`): `Standard` (10 newest, then hourly,
  daily, weekly, monthly), `More` (25, then every quarter hour, four hours, day, week) or `Fewer`
  (5, then every six hours, week, month, year). Standard is the policy every earlier build used,
  byte for byte (`a_retention_preset_keeps_more_or_fewer_versions_and_standard_writes_what_it_always_did`);
  a preset changes only which versions the next save drops, never the format; unlisted and kept always stay, a kept version holds its tier slot, and only the size
  limit can still drop a kept version, oldest first. A delta whose newer neighbour was dropped is
  recompressed against the newest kept version before it (stored whole when a dropped version was
  whole); one that cannot be decoded is copied as it was. When a delta could not be decoded for
  want of memory the versions dropped before it are kept; if a second pass still runs short the
  save fails (`EncodeError::HistoryTooLarge`) rather than write a delta whose neighbour is gone.
- Size: a model file stays within the `MAX_FILE_SIZE` every reader enforces. When versions would
  take it past three quarters of that, the oldest are dropped (a tail) and the save says how many
  (`Encoded::dropped_for_size`). A model too large on its own is refused
  (`EncodeError::ModelTooLarge`), so are records whose uncompressed snapshot exceeds
  `MAX_DECOMPRESSED` (exported as `MAX_MODEL_RECORDS`), both checked before anything is written.
- Versions copied unchanged are block-aligned (a zero-filled `Padding` chunk, skipped by readers)
  so `copy_file_range` (`os/unix.rs`) or ReFS block cloning (`os/windows.rs`) can clone them;
  what could not be cloned, or the file having changed since it was read, is written from memory.

## Saving

- Write a temporary sibling, fsync, rename over the target, fsync the directory (a failed
  directory fsync is logged, not a failed save). On Windows the rename is `ReplaceFileW` with no
  directory fsync (`windows.md`). Permissions, group and extended attributes (ACLs too) are kept;
  the temporary is owner-only until applied, and a new file gets the umask's mode.
  `write_privately` is the same write for secrets (the cloud sign-in when no keyring takes it): a new or replaced file is
  owner-only. A symbolic link is followed.
- Temporaries are `.<name>.<machine>-<boot>-<pid>-<n>.tmp`. After each write, those next to the
  file are removed when from an earlier boot of this machine, or from this boot with their process
  gone; other machines' saves on a shared folder are left alone. The recovery-directory scan
  sweeps the same way.
- Names past `MAX_NAME_BYTES` (temporaries, `.damaged` copies) are cut and end in a hash
  (`paths::fitting`); a model whose name leaves no room for `.<name>.journal` keeps its journal in
  the recovery directory.
- `SaveOptions::unless_changed_from` (the head digest the session loaded or last saved) makes a
  save refuse with `SaveError::ChangedOnDisk` when the file it replaces now has another head,
  leaving it as it was; replacing anyway makes the outside head the newest version. Since encoding
  can take seconds, a guarded save checks again before making a backup and in the read-back just
  before the rename (`Previous::ensure_still_at`): the file read must still be the target
  (`FileKey`, which catches a rename-replace that leaves the read handle's stamp alone, on Windows
  too) with its `ChangeStamp` unchanged, and a target that was absent must still be; a hard-linked
  backup changes the ctime, so the stamp is taken again after it. `set_version_kept` makes the same
  read-back check, guarded or not. The instant between that check and the rename cannot be closed
  portably.
- A save refuses when the earlier versions in the file it replaces cannot be read, and when the
  file is read-only; the notice offers Save As.
- Before the rename, a model save reads the synced temporary back and checks that it parses
  undamaged, its head digest is the one encoded and every record hashes to it
  (`binary::reads_back`); otherwise the temporary is removed and the file left as it was, as on a
  write failing halfway (`save::tests`: `a_save_failing_halfway_through_writing_leaves_the_file_as_it_was`,
  `a_save_that_does_not_read_back_leaves_the_file_as_it_was`). This guards against the compressor
  and the shared-range copies. Every version data chunk the save compressed itself
  (`Encoded::fresh_versions`) is rebuilt from the written file through its chain and checked
  against the digest of what was compressed; versions copied as stored are decoded only as links
  of such a chain. A failure is `NotReadBack::Version`, reported as
  `WriteFailure::VersionNotReadBack`.
- Overwriting a file that loaded with problems first keeps the original as
  `<name>.damaged.caditor` (`keep_copy`, also used for unreadable preferences and recent files): a
  hard link, else a copy moved into place whole. The same copy is kept when the file being
  replaced has gone bad since it was loaded (`Encoded::previous_damaged`).

## Clipboard (`clipboard.rs`)

- Text: a header line `caditor clipboard: <kind>, version <n>` (`ClipboardKind::SketchGeometry` or
  `Features`, `CLIPBOARD_VERSION`), then one line of JSON made of the model file's own records
  (`EntityRecord`, `ConstraintRecord`, `FeatureRecord`, each `Lenient`), so a damaged or unknown
  item is left out with a note and the rest pastes.
- Each payload names its `source` (copying process and document session, chosen by the app) and
  carries the parameters its expressions use by id, name and value (`CarriedParameters`, stored
  literal text); `PasteOrigin` is `ThisDocument` only when the source is the reader's own, and
  parameters are then kept by id (`document.md`).
- Sketch geometry is written from a scratch sketch the clip is pasted into and read back through
  `restore_sketch` and `Sketch::clip`, so loading's checks apply; dimensions whose parameters
  cannot be carried are left out with a note, and so is a text whose height or angle cannot be
  (its anchor point stays) or a gear with any expression that cannot (its centre stays). The payload names the sketch it was copied from, so
  pasting into it again can shift the copy.
- Reading refuses in words (`ClipboardError`): text without the header (`Foreign`), another or
  unknown kind, a newer version, damaged JSON, nothing usable, text over `MAX_CLIPBOARD_TEXT`. The
  header tolerates a byte-order mark and CRLF line ends.
