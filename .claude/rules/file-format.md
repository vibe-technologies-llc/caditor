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
  - Anything else that must not be lost needs a must-understand chunk.
  - Every shipped version keeps files in the format corpus, which proves it still reads.

## Format corpus (`corpus/`, `tests/corpus.rs`)

- `crates/caditor-file/corpus/<format>-<version>/` holds files caditor wrote in each shipped
  version of each format it owns: `model-1/` (every record kind, feature record kind, sketch entity
  and constraint kind and entity flag, `pieces.caditor` holding trimmed and broken pieces with their
  `split_from`, a version history with a kept version and block-alignment padding, sliced
  content, a thread and a tapped hole carrying `modelled`), `journal-1/` (every journaled edit,
  apply, undo and redo whole and by reference, a rebased snapshot, sliced content) and
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

- Head chunk (save time, last change's name, blake3 digest of records), one zstd chunk per record
  (`RECORD_KINDS`), version history. The journal snapshot carries `principal`, `suppressed` and
  `rollback` as fields.
- A record with unchanged understood content is written back exactly as stored (newer fields kept).
  New records are encoded first; an old record whose bytes hash like a new one's is matched by
  them, any other by the digest of its re-encoded understood content (`understood_digest`), so only
  changed records are parsed. A match by understood content whose bytes differ re-encodes around
  the stored ones (`NewRecords::keeping_understood`). Unknown chunk kinds are carried unless
  must-understand; loading reports those as left out, so the original is kept as `.damaged`.
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
  `name`, `opacity` (percent), `finish` (`Finish::key`: `matte`, `satin`, `glossy`, `rubber`,
  `powder_coat`, `anodized`, `brushed_metal`, `polished_metal`) and `faces` (face record with
  `colour`, optional `opacity` and `finish`), each optional. Unreadable part, unknown finish, or a
  face opacity outside `MIN_OPACITY_PERCENT` to 100 is dropped, reported. Journal:
  `set_body_appearance`. Written as `BodyAppearance::settled` gives it, so a journaled request the
  document settles (blank material, opacity 100 or above) reads back. `describe_unreadable_record` skips `FEATURE_FIELDS` when naming an unknown
  kind.
- Sketch `projections`, only when projected: each `entity` and `source` (`edge` or `section` with
  `body` and an edge record, `vertex` with `body` and the vertex name's digest, `sketch_entity`
  with `sketch` and `entity`, `datum_plane` with `datum` and a finite positive `reach`,
  `principal_plane` with `plane` `xy`/`xz`/`yz` and `reach`). The projected flags are derived on
  load after the constraints. Unknown or unreadable source: geometry kept as ordinary, reported.
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
  left out, each reported. The corpus holds it in `path-sweeps.caditor`.

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
  `face` (face record) or `body`; unreadable group left out.

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
- `mirror`: `body`, `plane` (unreadable YZ), `keep_original`. `feature_mirror`: `mirrored`, ids of
  the features reflected in tree order (repeated id kept once). `face_mirror`: `faces` (unreadable
  left out; none left mirrors the whole body, reported), around a `feature_mirror` when a mirror
  holds both (the features win when computed, the faces are kept).
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

Splits and faces:

- `split`: `body`, `plane` (unreadable YZ), `flipped` when set. `split_along`: `along` as
  `{"body": id}` or `{"sketch": id}`. `split_surface`: `body`, `face` attachment record; unreadable
  splits along YZ.
- `offset_face`: `body`, `distance` (unreadable 1 mm), `faces` (unreadable left in place),
  `tangent` when set. `split_face`: `body`, `faces` (unreadable left whole), `along`
  (`{"plane": ref}` unreadable YZ, `{"body": id}`, `{"sketch": id}`). Wrappers: `split_face_along`
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
  (`start`, `apex`, `end`, `rho`; an out-of-range rho is refused like any bad entity). A conic's
  rho dimension is a `rho` constraint (`conic`, `value`), falling back to the drawn rho.
- `ellipse` (`center`, `major`, `minor_radius`), `elliptical_arc` (plus `start`, `end`). Radii are
  `major_radius` and `minor_radius` constraints (`ellipse`, `value`, falling back to the drawn
  radius); `on_minor_axis` (`point`, `ellipse`). Naming an unknown kind skips the flags (`id`,
  `name`, `construction`, `split_from`, `inactive`, `label`).
- An entity record has `construction` when true and `split_from` (the id of the curve it was split
  from, `sketch.md`) when set, both left out otherwise, in the model and in a journaled
  `add_sketch_entity`. A `split_from` the sketch refuses (the entity itself) is reported and
  dropped, the curve kept.
- A reader not knowing one reports it as a kind from a newer version and keeps the points.

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
  age tiers; unlisted and kept always stay, a kept version holds its tier slot, and only the size
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
  the temporary is owner-only until applied. A symbolic link is followed.
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
  cannot be carried are left out with a note. The payload names the sketch it was copied from, so
  pasting into it again can shift the copy.
- Reading refuses in words (`ClipboardError`): text without the header (`Foreign`), another or
  unknown kind, a newer version, damaged JSON, nothing usable, text over `MAX_CLIPBOARD_TEXT`. The
  header tolerates a byte-order mark and CRLF line ends.
