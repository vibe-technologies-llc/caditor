# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

caditor is a parametric CAD application for Linux and Windows, written in Rust (edition 2024)
with wgpu for rendering, on its own B-rep kernel and constraint solver. Licensed AGPL-3.0-only.
User experience and never losing the user's work outrank everything else; see
`.claude/rules/ux.md` and `.claude/rules/reliability.md`.

## Commands

```sh
cargo run -p caditor
cargo test --workspace
cargo test -p <crate> <test_name>
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo xwin clippy --target x86_64-pc-windows-msvc --workspace --all-targets --all-features -- -D warnings
rust-formatter
cargo deny check
packaging/build-release.sh --snapshot
```

`rust-formatter` formats `.rs` and `.toml` and replaces `cargo fmt` and `rustfmt` entirely.
`cargo xwin` (cargo-xwin) lints the Windows build from Linux; its tests run under wine, except that
wine cannot rename over an open file, which journals rely on, so Windows CI is the real check.
Offscreen render tests skip without a GPU adapter unless `CADITOR_REQUIRE_GPU=1`. Screenshots of
the interface: the `ui-screenshots` skill. CI, cargo-deny and fuzzing: `.claude/rules/ci.md`.

## Architecture

The Cargo workspace is `crates/*`; dependencies point one way, listed here by what each crate uses
inside the workspace.

| Crate | Role | Uses |
| --- | --- | --- |
| `caditor-geometry` | f64 math vocabulary: `glam` aliases, `Plane`, `Ray`, `Aabb`, rigid transforms. | none |
| `caditor-expression` | `Quantity` (mm and degrees with a `Dimension`) and expressions over parameters. | none |
| `caditor-sketch` | 2D sketches on a `Plane`: entities, constraints and its own solver. | expression, geometry |
| `caditor-kernel` | Own B-rep kernel (no truck, no OpenCascade): curves, surfaces, topology, tessellation, naming, profiles, booleans, blends, shells, patterns. | geometry |
| `caditor-step` | STEP (ISO 10303-21, AP214) writing and reading of kernel solids. | kernel, geometry |
| `caditor-zstd` | Safe wrapper over the pure-Rust zstd port; one of the two crates with `unsafe`. | none |
| `caditor-windows` | Safe wrappers over the Win32 calls nothing else offers safely; the other crate with `unsafe`, empty off Windows. | none |
| `caditor-document` | The parametric model: parameters, feature tree, transactions, undo, recompute worker. | expression, geometry, kernel, sketch |
| `caditor-file` | Persistence: binary container, version history, recovery journal, preferences, DXF/SVG/STEP import, STL/3MF/STEP/PNG export. | document and everything it uses, step, zstd, windows |
| `caditor-render` | wgpu viewport, camera, GPU picking, image export; no winit or document dependency. | geometry |
| `caditor` | The winit/egui application: UI, commands, sketch editing, file workflow. | all but step and zstd |

Platform code is a pair of `cfg(unix)` and `cfg(windows)` items behind one interface; Windows
specifics (the Win32 boundary, paths, saving, locks, dialogs, the MSI) are in `windows.md`.

Invariants across crates:

- The world is Z-up and model data is f64; conversion to f32 happens only at the GPU boundary.
- Entities, constraints, parameters and features have stable IDs (`EntityId`, `ConstraintId`,
  `ParameterId`, `FeatureId`) from a per-container counter: never reused, never positional.
  Generated topology is named from the feature that made it (`FaceName`, `EdgeName`). Anything
  referencing model geometry keeps this property, since positional naming is the root of FreeCAD's
  topological naming failures.
- The document changes only through `Document::apply` of a `Transaction`; the UI returns `Action`s
  and never mutates the model directly.
- Kernel operations are valid or an error, never a bad solid or a panic.

## Releases

A `v<version>` tag builds a `.tar.zst` with an installer on Ubuntu 22.04 and a per-user MSI on
Windows (`release.yml`) and publishes a GitHub release. `packaging/` holds the desktop entry,
logo, metainfo, installer and release scripts, `packaging/windows/` the WiX source and its
scripts; `docs/RELEASING.md` explains the choices and the steps.

## Roadmap

`docs/TODO.md` holds the roadmap and open design decisions; check it before starting new work. It
lists only what remains: the change that implements an item deletes it, and a resolved decision is
removed once recorded in the rules file covering it.

## Rules

Design detail lives in `.claude/rules/`, each file loading only for the paths it governs. A change
that makes a rule false updates it in the same commit.

| File | Covers |
| --- | --- |
| `ux.md`, `reliability.md` | UX requirements and the FreeCAD failure modes to avoid; crash and data-loss policy (always loaded). |
| `rust-style.md`, `dependencies.md`, `ci.md` | Formatting, imports, collections, `unsafe`; dependency declaration; CI, cargo-deny, fuzzing. |
| `expression.md`, `zstd.md`, `windows.md` | Quantities, units and the expression language; the zstd wrapper's `unsafe` boundary; Windows: the Win32 boundary, files, app and MSI. |
| `sketch.md`, `sketch-solver.md` | Sketch entities, constraints and operations; the solver, DOF and conflict diagnosis. |
| `kernel.md`, `kernel-tessellation.md`, `kernel-naming.md`, `kernel-profile.md`, `kernel-intersect.md`, `kernel-operations.md` | Kernel base (tolerances, curves, surfaces, topology); tessellation; topology names and references; profile regions; intersections; extrude, revolve, booleans, blends, shells, patterns. |
| `step-write.md`, `step-read.md` | STEP writing; the Part 21 parser and `read_step`. |
| `document.md`, `document-recompute.md` | Transactions, undo, model parameters, every feature kind; recompute caching, failure containment, the worker. |
| `file-format.md`, `file-journal.md`, `file-import-export.md` | Container, version history, atomic saving; recovery journal, storage worker, preferences; DXF, SVG, STEP, STL, 3MF and PNG. |
| `render.md` | Frames, devices, graphics settings, precision, depth, picking, navigation. |
| `app.md`, `app-look.md`, `app-modelling.md`, `app-files.md`, `app-input.md`, `app-sketching.md`, `app-tests.md` | App shell, `Model` and `Action`s; theme, widgets, panels, feature tree; modelling tools; file workflow and preferences; input, commands and keymap; sketch editing; the headless UI test harness. |
