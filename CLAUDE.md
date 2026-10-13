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

`rust-formatter` formats `.rs` and `.toml` and replaces `cargo fmt`. `cargo xwin` lints the Windows
build from Linux; its wine tests cannot rename over an open file (journals rely on it), so Windows
CI is the real check. Offscreen render tests skip without a GPU adapter unless
`CADITOR_REQUIRE_GPU=1`. Interface screenshots: the `ui-screenshots` skill. CI, cargo-deny and
fuzzing: `ci.md`.

## Architecture

The Cargo workspace is `crates/*`; dependencies point one way, listed here by what each crate uses
inside the workspace.

| Crate | Role | Uses |
| --- | --- | --- |
| `caditor-geometry` | f64 math vocabulary: `glam` aliases, `Plane`, `Ray`, `Aabb`, rigid transforms. | none |
| `caditor-expression` | `Quantity` (mm and degrees with a `Dimension`) and expressions over parameters. | none |
| `caditor-sketch` | 2D sketches on a `Plane`: entities, constraints and its own solver; the unit-free dimension layout the canvas and drawing export share. | expression, geometry |
| `caditor-kernel` | Own B-rep kernel (no truck, no OpenCascade): curves, surfaces, topology, tessellation, naming, profiles, booleans, blends, shells, patterns. | geometry |
| `caditor-step` | STEP (ISO 10303-21, AP214) writing and reading of kernel solids. | expression, kernel, geometry |
| `caditor-zstd` | Safe wrapper over the pure-Rust zstd port; one of the three crates with `unsafe`. | none |
| `caditor-windows` | Safe wrappers over the Win32 calls nothing else offers safely; a crate with `unsafe`, empty off Windows. | none |
| `caditor-wayland` | Safe Wayland protocol work on winit's own connection (files dragged onto the window, the window's `xdg-foreign` export for portal dialogs); the third crate with `unsafe`, empty off Unix. | none |
| `caditor-document` | The parametric model: parameters, feature tree, transactions, undo, recompute worker. | expression, geometry, kernel, sketch, zstd |
| `caditor-file` | Persistence: binary container, version history, recovery journal, preferences, DXF/SVG/STEP and STL/OBJ/3MF mesh import, STL/3MF/OBJ/glTF/STEP/PNG and DXF/SVG drawing export. | document and everything it uses, step, zstd, windows |
| `caditor-notices` | Release tool: writes `THIRD-PARTY-NOTICES.txt` from `cargo metadata` and the font licences. | none |
| `caditor-render` | wgpu viewport, camera, GPU picking, image export; no winit or document dependency. | geometry (windows on Windows) |
| `caditor` | The winit/egui application: UI, commands, sketch editing, file workflow. | all but step, zstd and notices (wayland on Unix, windows on Windows) |

Platform code is a `cfg(unix)`/`cfg(windows)` pair behind one interface; Windows specifics are in
`windows.md`.

Invariants across crates:

- The world is Z-up and model data is f64; conversion to f32 happens only at the GPU boundary.
  Display mesh normals, a display attribute, are packed in the kernel (`kernel-tessellation.md`).
- Entities, constraints, parameters and features have stable IDs (`EntityId`, `ConstraintId`,
  `ParameterId`, `FeatureId`) from a per-container counter: never reused, never positional.
  Generated topology is named from the feature that made it (`FaceName`, `EdgeName`). Anything
  referencing model geometry keeps this property, since positional naming is the root of FreeCAD's
  topological naming failures.
- The document changes only through `Document::apply` of a `Transaction`; the UI returns `Action`s
  and never mutates the model directly.
- Kernel operations are valid or an error, never a bad solid or a panic.

## Releases

A `v<version>` tag builds (`release.yml`) a `.tar.zst` with installer on Ubuntu 22.04, a `.deb`, an
`.rpm` and an AppImage from that archive, and a per-user MSI on Windows, and publishes a GitHub
release. `packaging/` (and `packaging/windows/` for WiX) holds the sources and scripts;
`docs/RELEASING.md` has the choices and steps.

## Roadmap

`docs/ROADMAP.md` indexes what remains; entries sit beside it by kind: `FEATURES.md`, `BUGS.md`
(wrong results and refusals), `PERFORMANCE.md`, `PLATFORMS.md`, `CHECKS.md`, `DECISIONS.md`. Check
them before new work. They list only what remains: the change implementing an entry deletes it, a
new finding goes into its kind's file, a resolved decision is removed once recorded in the rules
file covering it.

## Rules

Design detail lives in `.claude/rules/`, each file loading only for its `paths:`. A change making a
rule false updates it in the same commit; a change to what the user sees updates its page of the
user guide (`crates/caditor/guide/`, `app.md`). `conventions_tests.rs` fails on a source file no
rule's `paths:` covers, a rules file with nothing under its title and one missing from this table.

| File | Covers |
| --- | --- |
| `ux.md`, `reliability.md` | UX requirements and the FreeCAD failure modes to avoid; crash and data-loss policy (always loaded). |
| `rust-style.md`, `dependencies.md`, `ci.md` | Formatting, imports, collections, `unsafe`; dependency declaration; CI, cargo-deny, fuzzing. |
| `expression.md`, `zstd.md`, `windows.md`, `wayland.md` | Quantities, units and the expression language; the zstd wrapper's `unsafe` boundary; Windows: the Win32 boundary, files, app and MSI; the Wayland boundary and files dropped on Wayland. |
| `sketch.md`, `sketch-solver.md` | Sketch entities, constraints and operations; the solver, DOF and conflict diagnosis. |
| `kernel.md`, `kernel-tessellation.md`, `kernel-naming.md`, `kernel-profile.md`, `kernel-intersect.md`, `kernel-operations.md` | Kernel base (the geometry crate, tolerances, curves, surfaces, topology); tessellation; topology names and references; profile regions; intersections; extrude, revolve, booleans, blends, shells, patterns. |
| `step-write.md`, `step-read.md` | STEP writing; the Part 21 parser and `read_step`. |
| `document.md`, `document-recompute.md` | Transactions, undo, model parameters, every feature kind; recompute caching, failure containment, the worker. |
| `file-format.md`, `file-journal.md`, `file-import-export.md` | Container, version history, atomic saving; recovery journal, storage worker, preferences; DXF, SVG, STEP and mesh import, model, image and drawing export, parameter files. |
| `notices.md` | The third-party notices file: which crates and fonts it lists, where the texts come from, how packages install it. |
| `render.md` | Frames, devices, graphics settings, precision, depth, picking, navigation. |
| `app.md`, `app-look.md`, `app-modelling.md`, `app-files.md`, `app-input.md`, `app-sketching.md`, `app-tests.md` | App shell, `Model` and `Action`s, the user guide; theme, widgets, panels, feature tree; modelling tools; file workflow and preferences; input, commands and keymap; sketch editing; the headless UI test harness. |
