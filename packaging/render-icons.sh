#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/render-icons.sh

Renders packaging/caditor.svg into packaging/icons/caditor-<size>.png at every
size in the hicolor theme the release installs, the same files the program
embeds for its window icon and logo. Run it after changing the SVG and commit
the PNGs with it. It needs rsvg-convert (librsvg).
EOF
}

fail() {
    echo "render-icons: $1" >&2
    exit 1
}

case "${1:-}" in
    -h | --help)
        usage
        exit 0
        ;;
    "") ;;
    *)
        usage >&2
        exit 2
        ;;
esac

command -v rsvg-convert >/dev/null 2>&1 || fail "rsvg-convert is not installed"

root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$root/packaging/icons"
for size in 16 24 32 48 64 128 256 512; do
    rsvg-convert -w "$size" -h "$size" "$root/packaging/caditor.svg" \
        -o "$root/packaging/icons/caditor-$size.png"
done
