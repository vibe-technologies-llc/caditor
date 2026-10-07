#!/bin/sh
cd "$CLAUDE_PROJECT_DIR" || exit 0
if ! cargo metadata --locked --offline --format-version 1 --manifest-path fuzz/Cargo.toml >/dev/null 2>&1; then
    echo "fuzz/Cargo.lock is behind the root dependencies: run 'cargo update -w' in fuzz/ and commit it." >&2
    exit 2
fi
