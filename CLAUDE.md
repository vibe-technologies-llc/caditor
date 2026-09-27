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
caditor-geometry  ←  caditor-sketch  ←  caditor-document  ←  caditor (bin)
       ↑                                                          │
       └──────────────  caditor-render  ←─────────────────────────┘
```

`caditor-expression` has no workspace dependencies; the sketch, document and app crates all use
it.

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
- **caditor-document**: the parametric model: parameters, the ordered feature tree and
  everything that changes or recomputes it.
  - Every mutation is a `Transaction` of `Edit`s passed to `Document::apply`, the only public
    mutator. `apply` is atomic and returns the inverse transaction, and `Editor` keeps undo and
    redo as stacks of these inverses. Edits carry their IDs, so redo restores the same IDs, and
    ID counters never move backwards. Edits refuse to break invariants: unknown references,
    parameter cycles, deleting something still in use, or moving a feature past one it depends
    on. `Document::check` runs a transaction on a clone so the UI can report the error before
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
