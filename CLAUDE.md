# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

caditor is a parametric CAD application for Linux, written in Rust (edition 2024) with wgpu for
rendering. Licensed AGPL-3.0-only.

The repository has no Cargo workspace yet. Once one exists, record the crate layout and the
data flow between the parametric model, the geometry kernel and the renderer here, and keep
detailed conventions in `.claude/rules/`.

## Commands

```sh
cargo build
cargo test
cargo test -p <crate> <test_name>
cargo clippy --workspace --all-targets
rust-formatter
rust-formatter --check
```

`rust-formatter` formats both `.rs` and `.toml` files. It replaces `cargo fmt` and `rustfmt`
entirely; see `.claude/rules/rust-style.md`.

## Rules

- `.claude/rules/rust-style.md`: formatting, imports, `unsafe`, edition and toolchain.
- `.claude/rules/dependencies.md`: how dependencies are declared and versioned.
