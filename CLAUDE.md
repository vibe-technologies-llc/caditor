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
- **caditor-sketch**: 2D sketches on a `Plane`: entities, constraints with stable
  `ConstraintId`s, dimensions whose values are expressions, `evaluate` (later the solver).
  `insert_entity` and `insert_constraint` take explicit IDs and check references, for loading.
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
    the undo history. The journal lives next to the file as `.<name>.journal`, falling back to
    `$XDG_STATE_HOME/caditor/recovery/`, where untitled documents keep theirs. Its owner holds
    an exclusive lock on it, which is how the startup scan and other instances tell a live
    journal from an orphan.
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
    Expression fields parse, evaluate and check the dimension before building a transaction.
  - `ui_tests.rs` drives the real panels through a headless egui context with synthetic input.

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
