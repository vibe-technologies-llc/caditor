# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

caditor is a parametric CAD application for Linux, written in Rust (edition 2024) with wgpu for
rendering. Licensed AGPL-3.0-only. User experience and never losing the user's work are the two
product priorities that outrank everything else; see `.claude/rules/ux.md` and
`.claude/rules/reliability.md`.

## Commands

```sh
cargo run -p caditor
cargo build --workspace
cargo test --workspace
cargo test -p <crate> <test_name>
cargo clippy --workspace --all-targets -- -D warnings
rust-formatter
rust-formatter --check
```

`rust-formatter` formats both `.rs` and `.toml` files. It replaces `cargo fmt` and `rustfmt`
entirely; see `.claude/rules/rust-style.md`.

## Architecture

The Cargo workspace is `crates/*`. Dependencies point in one direction only:

```
caditor-expression  ←──────────────────┐
       ↑                               │
caditor-geometry  ←  caditor-sketch  ←  caditor-document  ←  caditor-file  ←  caditor (bin)
       ↑                                                                          │
       └──────────────  caditor-render  ←─────────────────────────────────────────┘
```

`caditor-expression` has no workspace dependencies; the sketch, document, file and app crates all
use it. `caditor-file` and the app also use the geometry and sketch crates directly.

- **caditor-geometry**: the math vocabulary, as f64 `glam` aliases (`Point3`, `Rotation3`, …)
  plus `Plane` (origin, normal and in-plane x axis), `Ray` and `Aabb`. The world is Z-up and
  model data is f64 throughout; conversion to f32 happens only at the GPU boundary in the
  renderer.
- **caditor-expression**: units and expressions. A `Quantity` is an f64 in base units
  (millimetres and degrees) with a `Dimension` of length and angle powers. A plain number takes
  the dimension of whatever it is added to, and a field that expects a length takes a plain
  result as millimetres. Trigonometry reads a plain number as radians. An `Expression` refers to
  parameters by `ParameterId`, never by name, so renaming a parameter rewrites every
  expression's text. Parsing limits length and nesting so that hostile input cannot overflow the
  stack, and errors are plain-language clauses.
- **caditor-sketch**: 2D sketches on a `Plane` and caditor's own constraint solver.
  - Entities are points, lines, circles (centre point and radius), arcs (centre, start and end
    points, counter-clockwise) and clamped B-splines through control points. Every sketch also
    has a fixed origin and two axes under reserved IDs (`EntityId::ORIGIN`, `HORIZONTAL_AXIS`,
    `VERTICAL_AXIS`) that the counter never reaches; stored IDs stay below 2^63.
  - Constraints have stable `ConstraintId`s: coincident (point–point or point on a curve),
    horizontal, vertical, parallel, perpendicular, tangent, equal, and the dimensions distance,
    angle and radius, whose values are expressions. `check_constraint` refuses constraints that
    do not fit the entity kinds, so the UI can ask before offering one. `insert_entity` and
    `insert_constraint` take explicit IDs and check references, for loading.
  - `solve` evaluates the dimensions, then runs damped Gauss–Newton with minimal-norm steps
    (SVD from `nalgebra`) on each independent part of the system, so geometry that already
    satisfies its constraints does not move and under-constrained geometry moves as little as
    possible. Every equation has an analytic gradient; two-branch equations (tangent side,
    signed distance) take their branch from the starting geometry, so a solve never flips.
    Degrees of freedom and each entity's constraint state come from the rank and null space
    of the Jacobian at the solution; a constraint whose equations add no rank over older ones
    is reported as redundant, naming what it duplicates. When a part does not converge, a
    deletion filter finds a minimal set of conflicting constraints, which recompute reports as
    the feature's error with `FeatureError.constraints` and `FixTarget::Constraint`.
- **caditor-document**: the parametric model: parameters, the ordered feature tree and
  everything that changes or recomputes it.
  - Every mutation is a `Transaction` of `Edit`s passed to `Document::apply`, the only public
    mutator of content (`reserve_ids_below` only raises the ID counters, for loading). `apply`
    is atomic and returns the inverse transaction, and `Editor` keeps undo and redo as stacks of
    these inverses. Edits carry their IDs, so redo restores the same IDs, and ID counters never
    move backwards. Edits refuse to break invariants: unknown references, parameter cycles,
    deleting something still in use, or moving a feature past one it depends on.
    `Document::check` runs a transaction on a clone so the UI can report the error before
    committing.
  - Sketch content changes only through sketch edits (add, remove or set an entity, add or
    remove a constraint, set a dimension). Removing an entity that something still uses is
    refused rather than cascaded; `TransactionBuilder::remove_sketch_items` expands a user's
    deletion into constraints first, then curves, then points. Setting an entity changes only
    its value, never its kind or the points it uses. `settle_sketch` moves the definition to a
    solved shape so the next solve starts from what the user sees.
  - Recompute: `ParameterValues` evaluates parameters in dependency order and reports cycles
    rather than following them. `Recompute` walks the features in tree order and reuses a
    cached result when the feature definition (an `Arc`, compared by pointer first), the values
    and names of the parameters it uses, and its upstream feature results are all unchanged. A
    failing feature is `Failed` with a `FeatureError` (reason, remedy and a `FixTarget`) and
    keeps its last good result. Its dependents fail with a pointer back to it, and everything
    else is unaffected. A panic inside an `Evaluator` is caught and becomes that feature's
    error.
  - `Recomputer` runs recompute on a worker thread. A newer submission or `cancel` stops the
    running job between features (evaluators also receive a `CancelToken`), and features that
    were not reached are reported as `Outdated`. The worker calls a wake callback after each
    report so the UI can redraw.
- **caditor-file**: persistence. A model file (`.caditor`) is UTF-8 JSON Lines: a header
  `{"format":"caditor","version":N}`, one self-contained record per parameter and per feature
  carrying its stable ID, and the ID counters. Expressions are stored as canonical text that
  refers to parameters as `$<id>` (`Expression::to_stored_text` and `parse_stored`), so stored
  text never depends on names, and numbers round-trip exactly. Every format version that has
  shipped stays readable.
  - Saving writes a temporary sibling, fsyncs it, renames it over the target and fsyncs the
    directory, keeping the target's permissions. Overwriting a file that loaded with problems
    first keeps the original as `<name>.damaged.caditor`.
  - Loading is partial. Each line and each sketch item is read on its own (`Lenient`), and the
    pieces are assembled through `Document::apply`, so a loaded model always satisfies the
    document invariants. Damaged or unknown (newer) records are left out, a lost parameter that
    something still uses becomes a stand-in with value 0, unusable or duplicate names are
    renamed, a parameter cycle is broken at the parameter that closes it, and an unreadable
    dimension takes its drawn length. Each of these is reported in plain language.
  - The recovery journal is JSON Lines as well: a header naming the file, a snapshot of the last
    saved state, then one entry per change (`apply`, `undo` or `redo` with the transaction that
    was applied), each line carrying a CRC32 of its entry. Replay stops at the first bad line,
    so a torn tail loses only the changes after it, and replaying through an `Editor` restores
    the undo history. New edit kinds do not bump the journal version: an older reader stops at
    the first entry it cannot read and keeps everything before it, whereas a newer version
    number would make it refuse the whole journal. The journal lives next to the file as
    `.<name>.journal`, falling back to `$XDG_STATE_HOME/caditor/recovery/`, where untitled
    documents keep theirs. Its owner holds an exclusive lock on it, which is how the startup
    scan and other instances tell a live journal from an orphan.
  - `Storage` is one worker thread per open document. It owns the journal and performs saves,
    so appends, saves and the rebase of the journal onto the saved snapshot stay in order, and
    it fsyncs after each batch of entries. A `Flusher` lets the panic hook wait for pending
    entries.
  - Recovery (`scan`, `journal_for`) inspects unlocked journals in the recovery directory and
    next to recent files, deletes those with nothing to recover (no net change, or already in
    the file) and returns the rest with a replayed `Editor`.
- **caditor-render**: wgpu device and surface ownership, the camera and the viewport. It does
  not depend on winit or on the document: it takes any `Arc<dyn WindowTarget>` and draws a
  `Scene` of lines, markers, convex fills and a grid built by the app. `begin_frame` draws the
  3D viewport into its rect and hands back a `Frame` whose encoder the app draws the UI into;
  `submit` presents it.
  - Precision: every position is converted relative to the eye in f64 before the cast to f32,
    and the view matrix is rotation only, so geometry far from the origin stays exact.
  - Depth is reverse-Z with an infinite far plane and `Depth32Float`, with 4x MSAA when the
    adapter supports it. Model geometry draws over reference geometry (datum planes, axes)
    through a per-`Layer` depth bias.
  - Picking renders a small window around the cursor into ID and depth targets and reads it
    back asynchronously, so hover never blocks the UI thread. Hits carry their world position,
    which navigation uses as the orbit pivot, pan grab point and zoom anchor.
  - Navigation has a single model: right-drag orbits (turntable around world Z), middle-drag or
    Shift+right-drag pans, the wheel and pinch zoom toward the point under the cursor, and
    view changes from the view cube or fit animate.
- **caditor**: the winit `ApplicationHandler` (`app.rs`), the egui integration drawn over the
  viewport (`overlay.rs`), the side panel (`panels.rs`) with the feature tree
  (`feature_tree.rs`) and parameter table (`parameter_table.rs`), the toolbar with undo, redo
  and recompute status (`toolbar.rs`), the viewport widget with navigation, hover and selection
  (`viewport.rs`), the view cube (`view_cube.rs`) and the conversion of documents and results to
  a `Scene` (`scene.rs`). Selectable things are `Pickable` values built from stable IDs.
  - `Model` (`model.rs`) owns the `Editor` and the `Recomputer`. The UI gets `&Model` and
    returns `Action`s, which the app performs after the UI pass, so the UI never mutates the
    document directly. Each change submits a snapshot to the worker. Feature geometry is drawn
    from the last good result, tinted when the feature failed or is outdated.
  - `Model` also owns the file session: the path, the last saved document (the model is
    unsaved exactly when its document differs from it), the journal entries since then and the
    `Storage` worker, to which every change is recorded. `files.rs` is the file workflow: the
    File menu and shortcuts, native dialogs through the XDG desktop portal (`rfd`) on their own
    thread, loading and recovery scans on a background worker, the unsaved-changes prompt before
    New, Open, Restore and Quit, the recovery offer and the load report. `main.rs` installs the
    panic hook that flushes the journal.
  - Every numeric input is a `field::commit_field`: it commits on Enter or loss of focus,
    reverts on Escape, and keeps invalid text with its error inline instead of discarding it.
    Expression fields parse, evaluate and check the dimension before building a transaction;
    sketch dimensions go through `field::dimension_transaction`, which also applies the
    constraint's own rule (a radius above zero). Viewport and toolbar shortcuts run only when
    no widget held keyboard focus at the start of the frame or the end of the previous one, so
    Escape or Enter in a field never reaches the viewport.
  - Sketch editing is a context, not a mode: `editing.rs` holds which sketch is edited and the
    active `Tool`, changed by `Action::Editing` commands that `app::perform` routes after the UI
    pass; it ends by itself when the sketch disappears or another document is opened. The
    viewport watches it: entering turns the camera to face the sketch plane and fits it, the
    grid moves to that plane, the sketch's origin and axes become pickable references, other
    features are dimmed and unpickable, and the selection keeps only that sketch. Clicks go to
    selection unless the tool `draws`. Escape backs out one step at a time: plane choice, shape
    in progress, tool, selection, then editing.
  - Drawing tools (`drawing.rs`: point, line, rectangle, circle, arc, spline) keep their clicked
    points, hover and arc sweep as viewport UI state and build one transaction per finished
    shape (`Draw line`, …), settled first like any sketch transaction. Lines chain, each new
    line joined to the last end by `Coincident`, until Escape or a click on the last point;
    splines finish on Enter or a click on the last control point. An arc runs the way the
    pointer swept around its centre, and its end is projected onto the circle through its
    start. Every inferred constraint is checked with `Sketch::check_constraint` on a shadow of
    the sketch and skipped if refused.
  - Snapping (`snap.rs`) runs on the UI thread against the displayed sketch, in screen space
    through the view: the shape's own pending point first, then existing points and the origin
    within 8 logical pixels, then lines, circles, arcs and the axes within 6, projecting onto
    the curve. A snapped point gets a `Coincident` with its target. A line end that snapped to
    nothing becomes exactly horizontal or vertical within 3° or 6 pixels and gets that
    constraint. The preview, snap marker and snap label are drawn from this state, and the
    snap target replaces the GPU hover while a drawing tool is active.
  - `sketch_tools.rs` turns the selection into candidate constraints checked by
    `Sketch::check_constraint`; `sketch_toolbar.rs` offers them as buttons and Shift+letter
    shortcuts, disabled with what to select, and the drawing tools on plain letters (P, L, R,
    C, A, S). Dimensions start at the value measured on the
    displayed geometry. Every sketch transaction first settles the sketch to the last result,
    but only when that result is up to date (`Model::settled_sketch`). The UI never solves; it
    reads constraint states, degrees of freedom and redundancies from the last evaluation
    (`sketch_status.rs`, colouring in `scene.rs`). `scene::displayed_sketch` is the definition
    with solved positions wherever the last result has the same entity.
  - The edited sketch is annotated over the viewport with the egui painter (`annotations.rs`,
    placement in `annotation_layout.rs`), from the displayed geometry projected through the
    current view, with offsets and sizes in screen points and no stored positions. Distances
    between points are parallel dimension lines with extension lines, point–line distances are
    perpendicular, angles are arcs at the lines' intersection (between their closest ends when
    nearly parallel), radii are leaders with an `R` prefix; dimensions sit away from the
    sketch's centre. Other constraints are glyphs stacked beside each constrained entity on the
    opposite side, painted as shapes or as letters the default fonts carry. Labels show the
    expression in the document's naming, followed by its value when it is not a literal.
    Conflicting and redundant constraints take the error and warning colours.
  - Labels and glyphs are `Pickable::SketchConstraint`: hovering highlights the constrained
    entities, clicking selects (Shift or Ctrl toggles), and Delete removes selected constraints
    and entities in one transaction. They are painted but not interactive while a drawing tool
    is active. Double-clicking a label opens an inline `commit_field` on the canvas with the
    value selected; so does a new dimension from the constraint tools and any
    `Focus::Dimension` of the edited sketch, which the app takes from the panels and hands to
    the viewport, waiting until the dimension can be drawn.
  - `ui_tests.rs` drives the real toolbars, panels and viewport through a headless egui context
    with synthetic input; picking needs the GPU, so tests set the viewport selection directly,
    while drawing tests click sketch positions mapped to the screen through the view and
    annotation tests click the painted labels and glyphs.

Entities, constraints, parameters and features are referred to by stable IDs (`EntityId`,
`ConstraintId`, `ParameterId`, `FeatureId`). IDs come from a per-container counter and are never
reused, never positional, and survive the removal of anything else. Anything that references
model geometry must keep this property, because positional naming is the root of FreeCAD's
topological naming failures.

## Roadmap

`docs/TODO.md` holds the roadmap, the open design decisions and the project's direction. Check
it before starting new work. It lists only what remains: the change that implements an item
deletes it (never ticks it), and a resolved decision is removed once it is recorded where it
belongs, such as the Architecture section or a rules file.

## Rules

- `.claude/rules/ux.md`: UX requirements and the FreeCAD failure modes to avoid.
- `.claude/rules/reliability.md`: crash and data-loss policy, panic lints.
- `.claude/rules/rust-style.md`: formatting, imports, comments, collections and locks, `unsafe`,
  edition.
- `.claude/rules/dependencies.md`: how dependencies are declared and versioned.
