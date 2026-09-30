# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

caditor is a parametric CAD application for Linux, written in Rust (edition 2024) with wgpu for
rendering, on its own B-rep kernel and constraint solver. Licensed AGPL-3.0-only. User experience
and never losing the user's work outrank everything else; see `.claude/rules/ux.md` and
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
cargo deny check
(cd fuzz && cargo +nightly fuzz run <target> corpus/<target> seeds/<kind> -- -dict=dictionaries/<kind>.dict -max_total_time=60)
packaging/build-release.sh --snapshot
packaging/check-install.sh target/dist/caditor-<version>-snapshot-linux-x86_64.tar.zst
```

`rust-formatter` formats `.rs` and `.toml` and replaces `cargo fmt` and `rustfmt` entirely.
Offscreen render tests skip without a GPU adapter unless `CADITOR_REQUIRE_GPU=1` (CI sets it, on
lavapipe). CI, cargo-deny and the fuzz workspace are described in `.claude/rules/ci.md`.

## Architecture

The Cargo workspace is `crates/*`. Dependencies point in one direction only:

```
caditor-expression  ←──────────────────┐
       ↑                               │
caditor-geometry  ←  caditor-sketch  ←  caditor-document  ←  caditor-file  ←  caditor (bin)
   ↑   ↑                                   │                  ↑  ↑                │
   │   └──────────────  caditor-render  ←──┼──────────────────┼──┼────────────────┘
   └──  caditor-kernel  ←──────────────────┘                  │  caditor-zstd
              ↑                                               │
              └───────────────────────  caditor-step  ←───────┘
```

`caditor-expression` has no workspace dependencies; the sketch, document, file and app crates use
it. `caditor-file` and the app also use the geometry and sketch crates directly. `caditor-kernel`
depends only on `caditor-geometry`, never on the sketch or document crates. `caditor-zstd` has no
workspace dependencies and only `caditor-file` uses it; `caditor-step` depends only on the kernel
and geometry crates and only `caditor-file` uses it.

| Crate | Role |
| --- | --- |
| `caditor-geometry` | Math vocabulary: f64 `glam` aliases (`Point3`, `Rotation3`, …), `Plane` (origin, normal, in-plane x axis; the frame of every circle and rotational surface), `Ray`, `Aabb`, `Aabb2`, `RigidTransform`, `RigidTransform2`. |
| `caditor-expression` | `Quantity` (f64 in mm and degrees with a `Dimension`) and expressions over parameters by `ParameterId`. |
| `caditor-sketch` | 2D sketches on a `Plane`: entities, construction geometry, constraints and caditor's own solver. |
| `caditor-kernel` | caditor's own B-rep kernel (no truck, no OpenCascade): curves, surfaces, topology, validation, tessellation, naming, profiles, intersections, sweeps, booleans, blends, shells, patterns. |
| `caditor-step` | STEP (ISO 10303-21, AP214) writing and reading of kernel solids. |
| `caditor-zstd` | Safe wrapper over the pure-Rust zstd port; the only crate with `unsafe`. |
| `caditor-document` | The parametric model: parameters, the feature tree, transactions, undo, recompute on a worker. |
| `caditor-file` | Persistence: binary zstd/xxh3 container, model files with version history, recovery journal, preferences, DXF and STEP import, STL/3MF/STEP and PNG export. |
| `caditor-render` | wgpu device and surface, camera, viewport drawing of a `Scene`, GPU picking, tiled offscreen image export; no winit or document dependency. |
| `caditor` | The winit/egui application: UI, commands, sketch editing, file workflow. |

Invariants that hold across crates:

- The world is Z-up and model data is f64 throughout; conversion to f32 happens only at the GPU
  boundary in the renderer.
- Entities, constraints, parameters and features are referred to by stable IDs (`EntityId`,
  `ConstraintId`, `ParameterId`, `FeatureId`) from a per-container counter: never reused, never
  positional, surviving the removal of anything else. Generated topology is named from the
  feature that made it (`FaceName`, `EdgeName`). Anything referencing model geometry keeps this
  property, since positional naming is the root of FreeCAD's topological naming failures.
- The document changes only through `Document::apply` of a `Transaction`; the UI returns
  `Action`s and never mutates the model directly.
- Kernel operations are valid or an error, never a bad solid or a panic.

## Releases

caditor ships as a `.tar.zst` per release with an installer, built on Ubuntu 22.04 by
`.github/workflows/release.yml` when a `v<version>` tag is pushed, and published as a GitHub
release. `packaging/` holds the desktop entry, the logo (`caditor.svg`, rendered to `icons/` by
`render-icons.sh`), metainfo, MIME type, installer, cargo-about licence template,
`build-release.sh` and a local-only Arch `arch/PKGBUILD`; `docs/RELEASING.md` explains the
choice and the steps.
`CHANGELOG.md` lists every release: a change users notice adds a line under `## [Unreleased]` in
the same commit, written for users.

## Roadmap

`docs/TODO.md` holds the roadmap, the open design decisions and the project's direction; check it
before starting new work. It lists only what remains: the change that implements an item deletes
it (never ticks it), and a resolved decision is removed once recorded in the rules file covering
it.

## Rules

Design detail lives in `.claude/rules/`, each file loading only for the paths it governs. A change
that makes a rule false updates it in the same commit.

| File | Covers |
| --- | --- |
| `ux.md` | UX requirements and the FreeCAD failure modes to avoid (always loaded). |
| `reliability.md` | Crash and data-loss policy, panic lints, persistence guarantees (always loaded). |
| `rust-style.md` | Formatting, imports, comments, collections and locks, `unsafe`, edition. |
| `dependencies.md` | How dependencies are declared and versioned. |
| `ci.md` | CI jobs and pins, `cargo deny`, the fuzz workspace, targets, seeds and dictionaries. |
| `expression.md` | `Quantity`, units, plain numbers, the function set, parse and evaluation limits. |
| `zstd.md` | `caditor-zstd`: prefix deltas, frame checks, the `unsafe` boundary. |
| `sketch.md` | Sketch entities, reserved IDs, construction geometry, constraints, splines. |
| `sketch-solver.md` | `solve`, `solve_dragging`, per-part scales, branches, DOF and redundancy, conflict diagnosis, `SolveMemo`. |
| `kernel.md` | Kernel base: cancellation, tolerances, curves, surfaces, topology, validation, `find_crossing`. |
| `kernel-tessellation.md` | Face triangulation, grid density, poles and pinches, `Mesh`, retries, `MAX_POINTS`. |
| `kernel-naming.md` | Face, edge and vertex names, `FaceOrigin`, face and edge references and their resolution. |
| `kernel-profile.md` | Profile arrangement, regions, `PieceId`, `RegionKey`, depth, selection. |
| `kernel-intersect.md` | Curve and surface intersections, coincidence, marching, point classification. |
| `kernel-operations.md` | Extrude, revolve and `Plan`, booleans, blends, shells, patterns. |
| `step-write.md` | `write_step`: product structure, exact STEP forms, text and number encoding. |
| `step-read.md` | Part 21 parser and `read_step`: units, precision, assemblies, geometry, healing, limits. |
| `document.md` | Transactions, undo, `Editor`, hidden flags, sketch edits, every feature kind. |
| `document-recompute.md` | Recompute caching, failure containment, body states, display data, the `Recomputer` worker. |
| `file-format.md` | Container, values, model records, version history and retention, atomic saving, reading limits, partial loading. |
| `file-journal.md` | Recovery journal, locks, `Storage` worker, preferences, recent files, recovery scan. |
| `file-import-export.md` | DXF and STEP import, STL, 3MF, STEP and PNG export. |
| `render.md` | Frames, devices, graphics settings (vsync, MSAA, shading) and device loss, precision, meshes, depth, lines, projection, picking, image export, navigation. |
| `app.md` | App shell, `Model` and `Action`s, body meshing, vertices and mass properties, the measure tool, offers, file session, signals, redraws and frame pacing, samples, CLI, accessibility. |
| `app-look.md` | Fonts, theme tokens, widgets and tabs, screen-reader naming, bars and panels, the title bar and window frame, window and panel persistence. |
| `app-modelling.md` | Extrude, revolve, fillet, chamfer, shell, pattern and datum tools and panels, visibility, sketch placement. |
| `app-files.md` | File workflow, onboarding, export, image export and import dialogs, version history, the tabbed preferences with graphics settings, units. |
| `app-input.md` | Numeric fields, shortcut focus rules, commands and keymap, palette, shortcut editor, keyboard-only operation, typed points. |
| `app-sketching.md` | Sketch editing context, dragging and box selection, drawing tools, construction, snapping, constraint tools, annotations. |
| `app-tests.md` | The headless egui UI test harness. |
