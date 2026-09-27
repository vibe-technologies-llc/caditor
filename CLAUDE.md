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
caditor-geometry  ←  caditor-sketch  ←  caditor-document  ←  caditor (bin)
       ↑                                                          │
       └──────────────  caditor-render  ←─────────────────────────┘
```

- **caditor-geometry**: the math vocabulary, as f64 `glam` aliases (`Point3`, `Rotation3`, …)
  plus `Plane` (origin, normal and in-plane x axis), `Ray` and `Aabb`. The world is Z-up and
  model data is f64 throughout; conversion to f32 happens only at the GPU boundary in the
  renderer.
- **caditor-sketch**: 2D sketches on a `Plane`: entities, constraints and, later, the solver.
- **caditor-document**: the parametric model, meaning named parameters and the ordered feature
  tree.
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
  viewport (`overlay.rs`), the panels (`panels.rs`), the viewport widget with navigation,
  hover and selection (`viewport.rs`), the view cube (`view_cube.rs`) and the document to
  `Scene` conversion (`scene.rs`). Selectable things are `Pickable` values built from stable
  IDs. The app owns the `Document` and gives the UI read-only access to it.

Entities and features are referred to by stable IDs (`EntityId`, `FeatureId`). IDs come from a
per-container counter and are never reused, never positional, and survive the removal of
anything else. Anything that references model geometry must keep this property, because
positional naming is the root of FreeCAD's topological naming failures.

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
