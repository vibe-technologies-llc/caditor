#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "Usage: .github/run-ignored.sh <cargo test target arguments> -- <exact test names>" >&2
    exit 2
fi

target=""
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    target="$target $1"
    shift
done
if [ "$#" -eq 0 ]; then
    echo "run-ignored.sh: no test names after --" >&2
    exit 2
fi
shift
wanted=$#

log=$(mktemp)
trap 'rm -f "$log"' EXIT
cargo test --release --locked $target -- --ignored --exact --nocapture "$@" 2>&1 | tee "$log"

ran=$(sed -n 's/^test result: [a-zA-Z]*\. \([0-9]*\) passed.*/\1/p' "$log" | awk '{ total += $1 } END { print total + 0 }')
if [ "$ran" -ne "$wanted" ]; then
    echo "run-ignored.sh: $wanted tests were named but $ran ran; a test was renamed or removed" >&2
    exit 1
fi
