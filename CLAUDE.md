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
                                          caditor-render  ←  caditor (bin)
```

- **caditor-geometry**: the math vocabulary, as f64 `glam` aliases (`Point2`, `Point3`, …)
  plus `Plane`. Model data is f64 throughout; conversion to f32 happens only at the GPU
  boundary in the renderer.
- **caditor-sketch**: 2D sketches on a `Plane`: entities, constraints and, later, the solver.
- **caditor-document**: the parametric model, meaning named parameters and the ordered feature
  tree.
- **caditor-render**: wgpu device and surface ownership and frame lifecycle. It does not depend
  on winit: it takes any `Arc<dyn WindowTarget>`. `begin_frame` clears the viewport and hands
  back a `Frame` whose encoder the app draws into; `submit` presents it.
- **caditor**: the winit `ApplicationHandler` (`app.rs`), the egui integration drawn over the
  viewport (`overlay.rs`) and the panels (`panels.rs`). The app owns the `Document` and gives the
  UI read-only access to it.

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
