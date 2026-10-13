# Audit

High-confidence findings against commit `b1d3d3bd` (master, 2026-10-13). Depth was a
pass over the whole workspace, with the known remainder in `BUGS.md`, `PERFORMANCE.md`,
`FEATURES.md`, `DECISIONS.md`, `PLATFORMS.md` and `CHECKS.md` treated as already tracked
and not repeated here. Each item below was read in the current tree. Low-confidence
leads were dropped.

Verification commands for a fix, matching this repo: `cargo test -p <crate> <test>`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo deny check`,
and `rust-formatter`. The OpenGL finding is the one the Linux `check` job already runs:
`WGPU_BACKEND=gl LIBGL_ALWAYS_SOFTWARE=1 CADITOR_REQUIRE_GPU=1 cargo test --locked -p caditor-render --lib offscreen_tests`.

## Findings

Ordered by leverage (impact against effort, at high confidence).

| # | Finding | Category | Impact | Effort | Risk | Evidence |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Piece rows are not a real tree selection | Correctness | A body command can change the open feature | S | Low | `panels.rs`, `feature_tree.rs`, `visibility.rs` |
| 2 | The mesh pipeline does not build on OpenGL | Correctness | The 3D view cannot be created on the GL backend; CI `check` is red | M | Medium | `viewport.wgsl`, CI run 38435662052 |
| 3 | Recovery trusts the path stored in the journal | Security | Startup reads that path; Restore saves there later | M | Medium | `recovery.rs`, `model.rs` |
| 4 | CSV quoting does not stop a formula cell | Security | The documented control does not do what it says | S | Low | `csv.rs`, `file-import-export.md` |
| 5 | Two copies of skrifa are linked | Dependencies | `cargo deny check` fails; a second font parser is in the binary | S | Low | `Cargo.lock`, `deny.toml`, `dependencies.md` |
| 6 | Sketch frames rebuild the body-snap key every time | Performance | Cost grows with shown bodies while a sketch is open | S | Low | `body_snap.rs` |
| 7 | Every recompute glimpse rebuilds the same body set | Performance | A full recompute of a large import is quadratic in that set | S | Low | `recompute.rs` |

### 1. Keep a piece row selected, and do not retarget feature commands onto the open feature

- **Evidence**: A Bodies row stores the body's id (`crates/caditor/src/bodies_tree.rs`, the click calls `feature_tree::choose_among` with that id). For a body of each separate piece that id is a piece id (`FeatureId::piece`), not a feature id. `Document::feature` (`crates/caditor-document/src/document.rs`) looks the id up raw (`feature_list.rs` `get_by_id`) and does not find a piece. `PanelState::forget_missing` (`crates/caditor/src/panels.rs`) keeps a chosen id only when `document.feature` finds it, and `app.rs` calls it on every revision, so the next edit drops the piece row. `visibility::hideable_rows` uses the same `feature` filter, so Hide selection never sees a piece chosen only in the tree. `feature_tree::current_feature` also misses the piece id and falls through to the open feature; `targets` then, when the chosen ids do not contain that open feature, returns the open feature alone. Suppress, Hide or show feature, and Delete feature therefore change the open feature.
- **Impact**: Selecting a piece of a separating feature does not survive the next edit, with no notice. Hide selection of that row hides nothing. While another feature is open, Suppress, Hide or show feature, and Delete feature change that feature instead of the piece's maker. The row's own eye is unaffected: `visibility::toggle_body` already branches on `is_piece`. `body_selection::tree_bodies` already keeps a piece id.
- **Effort**: S.
- **Risk**: Low. A normal body row is still its feature id. The change is which id stays chosen and which feature those commands receive.
- **Confidence**: High.
- **Fix sketch**: Treat a chosen id as still present when `body_maker` exists. Give `hideable_rows` and `current_feature` the same `is_piece` branch `tree_bodies` has, so a piece row stands for that body. Add a UI test that selects a piece, edits something else, and checks the row is still chosen, and that Delete feature with another feature open does not delete that feature.

### 2. Stop the mesh shader loading the shadow depth texture

- **Evidence**: `crates/caditor-render/src/viewport.wgsl` binds `shadow_map` as `texture_depth_2d`. `bounced` reads it with `textureLoad`. `shade`, which `fs_mesh` and `fs_mesh_solid` call, calls `bounced` whenever a face carries a finish and rendered shading is on, so the load is in the mesh fragment shader even for a scene that never takes that branch. The Linux `check` job's OpenGL step (`WGPU_BACKEND=gl`, llvmpipe, `offscreen_tests`) fails while creating the pipeline labelled `meshes`: naga reports that WGSL `textureLoad` from depth textures is not supported in GLSL. Run [38435662052](https://github.com/vibe-technologies-llc/caditor/actions/runs/38435662052) on `b1d3d3bd` is that failure; the Vulkan tests and the Windows job in the same run succeeded. `textureSampleCompareLevel` of the same map, used for the shadow comparison, is a different operation and is not the line that fails.
- **Impact**: On the OpenGL backend the mesh pipeline is never created, so the 3D view cannot draw. `packaging/INSTALL.md` promises OpenGL, and CI treats that backend as a required check, which is red on master. The gamma-space resolve already listed in `BUGS.md` is a separate, narrower GL limit.
- **Effort**: M.
- **Risk**: Medium. Bounce reconstructs a position from that depth. A second colour target, or a load from a non-depth copy of the depth, has to match the comparison sampler's values or the bounce lighting shifts.
- **Confidence**: High.
- **Fix sketch**: Keep the depth comparison sampler for shadows. Feed `bounced` a sampled depth that the GLSL backend can load (a non-depth texture written in the light pass, or an equivalent naga accepts). The GL `offscreen_tests` step must pass, including a scene that bounces light. Do not weaken the step to skip the backend.

### 3. Bind a recovered journal to the file it actually belongs to

- **Evidence**: `recovery::inspect` (`crates/caditor-file/src/recovery.rs`) decodes the journal and, while deciding whether it is already saved, calls `load` on `contents.file`, the path stored in the journal header. `scan`, which startup runs, inspects every `*.journal` in the recovery directory, the adjacent journal of each recent file, and journals named by a marker. It does not check that the header path is the file the journal sits beside. `journal_for` does rebind that path when a model is opened; `scan` does not. `Model::restore` stores `Recovered.file` as the session path, so the storage worker journals beside it and the next save writes the recovered document there. The recovery card does show that path (`files/dialogs.rs`, the muted line under the change count).
- **Impact**: A journal caditor is willing to scan is enough to make startup parse whatever regular file the header names, before any prompt. Restore then makes that path the open model. Nothing requires an adjacent journal to name the model it sits next to. The card shows the path, so a person who reads it can refuse; the read itself is not gated on that.
- **Effort**: M.
- **Risk**: Medium. Fallback journals are named by a hash of the real path, and an untitled journal has no model path. Rejecting every header path that is not the journal's neighbour would drop a legitimate fallback recovery.
- **Fix sketch**: For a journal next to a model, keep it only when the header path is that model (same path or same file identity). For a `file-<hash>.journal`, keep it only when the hash matches the header path. Do not call `load` on any other path. Keep `journal_for`'s rebind. Test a header that names a different file: the scan does not read it, and Restore does not offer it.

### 4. Make a formula-looking CSV cell text, not only a quoted field

- **Evidence**: `csv_cell` (`crates/caditor-file/src/csv.rs`) quotes a cell that starts with `=`, `+`, `-` or `@`. Quoting is RFC 4180 syntax; after a spreadsheet parses the file, the cell value still starts with that character. `file-import-export.md` says the quotes exist so a spreadsheet never reads the cell as a formula. They do not. Parameter notes are free text, and a configuration name is any trimmed one-line string; both are written as cells. The parameter and configuration guides tell people to open these files in a spreadsheet. Parameter names cannot start with `=`.
- **Impact**: A note or a configuration name that starts with a formula character is still a formula when the exported CSV is opened. The control the format rule describes is not the control that works.
- **Effort**: S.
- **Risk**: Low. A text marker has to be stripped on import, or a note, a name, or a genuine negative expression comes back with the marker still in it. Corpus files that only check quoting need an update in the same change, and corpus files are write-once, so the change is a new behaviour of today's writer, not a rewrite of an old file.
- **Fix sketch**: When writing a cell whose value starts with a formula introducer, prefix a marker spreadsheets keep as text, and strip that same prefix when reading. Cover a note and a configuration name in the round-trip tests. Update the sentence in `file-import-export.md` so it describes the marker.

### 5. Link one skrifa

- **Evidence**: `dependencies.md` says skrifa is the copy epaint builds, so no second font parser is compiled. The workspace pins `skrifa` 0.48.0 (`Cargo.toml`), and `caditor-file` uses it for SVG text (`import/svg/font.rs`). `Cargo.lock` also has skrifa 0.44.0 on read-fonts 0.41.0, required by epaint 0.36.2, and skrifa 0.48.0 on read-fonts 0.45.0, required by `caditor-file`. `deny.toml` sets `multiple-versions = "deny"` and does not skip either crate. `cargo deny check bans` with cargo-deny 0.20.2 (the version CI pins) reports both duplicates. CI has not reached that step on `b1d3d3bd` because the OpenGL tests fail first; the check job runs `cargo deny --locked check` after them.
- **Impact**: The Linux and Windows graphs compile two font parsers. The dependency rule is false, and the dependency check fails as soon as the OpenGL step is green.
- **Effort**: S, if 0.48's API use in `font.rs` still builds against 0.44. M if it does not and the duplicate has to be an explicit skip until epaint moves.
- **Risk**: Low for a skip recorded with a reason. Medium for pinning `caditor-file` back to 0.44, which can change SVG outlines or kerning if 0.48 is load-bearing. Stop and report if the 0.44 build of `font.rs` does not compile, rather than widening the pin silently.
- **Fix sketch**: Prefer one version. If epaint's 0.44 cannot serve `font.rs`, add versioned `deny.toml` skips with that reason and rewrite the "no second font parser" sentence in the same change. `cargo deny check` must pass.

### 6. Rebuild the body-snap basis only when it can have changed

- **Evidence**: While a sketch is edited, each 3D-view frame refreshes body snaps. `BodySnaps::refreshed` (`crates/caditor/src/body_snap.rs`) always builds a `Basis` before comparing it with the one it kept. `Basis::of` walks every shown body, looks it up in the sketch's seen-body map, and clones each result `Arc` into a new vector. `same` then discards that vector when nothing changed. This is not the per-entity snap walk already listed in `PERFORMANCE.md`.
- **Impact**: Every frame with a sketch open does work proportional to the number of shown bodies, allocations included, even when the snap cache is still valid. On an assembly of a few thousand bodies that is paid on hover and orbit, on the UI thread, before any snapping.
- **Effort**: S.
- **Risk**: Low. The basis is a cache key. Skipping a rebuild that should have run snaps to a stale body until the next real change, so the cheap key has to include the edited sketch, its plane, and the evaluation identity the basis already compares.
- **Fix sketch**: Return before `Basis::of` when a cheap key matches the kept basis. Rebuild the body list only when that key changes. A test can refresh twice on an unchanged model and assert the generation does not move.

### 7. Build the glimpse body set once per recompute

- **Evidence**: `offer_glimpse` (`crates/caditor-document/src/recompute.rs`) runs once per feature that misses the cache, before that feature is computed. Each call collects `settling`'s body ids into a new `BTreeSet`, and it does that before the glimpse time budget is consulted. `settled_bodies` only needs the set to skip piece ids that are already listed; an ordinary body returns immediately.
- **Impact**: A full recompute (a first open, a mesh-quality change, or an edit that misses the cache all the way down) inserts that set once per such feature. On a few thousand imported bodies the set is the whole body list, so the walk is quadratic in it, on the recompute worker, including features for which no glimpse is sent. Headless export that skips display data does not take this path. The lead-screw time in `PERFORMANCE.md` is a different cost.
- **Effort**: S.
- **Risk**: Low. The set is a filter of piece ids. Building it once at the start of the walk has the same contents.
- **Fix sketch**: Compute the set once in the walk and pass it into `offer_glimpse`. A recompute of many separate bodies should not rebuild it per feature; an existing parallel or import test can count the constructions if a test seam already exists, and otherwise the structural change is the check.

## Direction

No new product direction. Assemblies, surfaces and sheet metal are already the open scope questions in `DECISIONS.md`, and the unfinished tools are already listed in `FEATURES.md`. The highest-leverage work that is not in the table above is still the tracked one: STEP bodies that import as facets (`BUGS.md`, STEP import) and booleans that fail on near-coincident tori and sub-resolution contacts (`BUGS.md`, Kernel). Those are hard, already described, and not re-planned here.

## Considered and rejected

- A timed-out or failed reach check counting as a touch, including `Reaches::most` returning the later body when nothing overlapped. `app-modelling.md` says a failed or timed-out check counts as a touch, and a forward touch is never turned around.
- The footprint test comparing boxes. The same page says the comparison is of boxes, and a profile that covers most of a face but not its extremes still cuts.
- The ignored `ttf-parser` advisory. `deny.toml` and `dependencies.md` already record that it arrives only through winit's window decorations.
- 3MF part paths, SVG and XML expansion limits, zenity and notify-send argument lists, and the zstd single-frame guard. Those checks are present and match the rules that describe them.
- The OpenGL resolve staying in gamma. Already in `BUGS.md`, and blocked on wgpu's GL backend. The mesh-pipeline failure above is the one that stops the view being created.
- Re-listing `PERFORMANCE.md`, `FEATURES.md` and `PLATFORMS.md`. They are the project's own remainder and were used as the baseline, not as new findings.

## What this pass did not do

Kernel boolean, shell, blend and STEP healing were not re-derived line by line; their known failures stay in `BUGS.md`. Fuzz targets were not re-run. No Windows desktop session was used. `cargo deny check bans` was run locally; the full `cargo deny check` advisories and licences pass was not, beyond that bans failure.
