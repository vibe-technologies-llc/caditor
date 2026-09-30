#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/check-install.sh ARCHIVE

Extracts a release archive built by packaging/build-release.sh, installs it
with its install.sh into a temporary prefix, checks every packaged file, the
menu entry and the program, then uninstalls it and checks nothing is left.
EOF
}

fail() {
    echo "check-install: $1" >&2
    exit 1
}

if [ $# -ne 1 ]; then
    usage >&2
    exit 2
fi
case "$1" in
    -h | --help)
        usage
        exit 0
        ;;
esac

archive=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
[ -f "$archive" ] || fail "$archive does not exist"
command -v desktop-file-validate >/dev/null 2>&1 || fail "desktop-file-validate is needed"
command -v zstd >/dev/null 2>&1 || fail "zstd is needed"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

zstd -dc "$archive" | tar -xf - -C "$work"
package=$(find "$work" -mindepth 1 -maxdepth 1 -type d)
[ -n "$package" ] || fail "the archive holds no directory"

prefix="$work/prefix & 100% sure"
"$package/install.sh" --prefix "$prefix" >/dev/null

(cd "$package" && find bin share -type f) | while IFS= read -r file; do
    [ -f "$prefix/$file" ] || fail "$file was not installed"
done
"$prefix/bin/caditor" --version >/dev/null || fail "the installed program does not run"

entry="$prefix/share/applications/caditor.desktop"
desktop-file-validate "$entry"
grep -qxF "TryExec=$prefix/bin/caditor" "$entry" || fail "TryExec does not name the program"
escaped=$(printf '%s' "$prefix/bin/caditor" | sed 's/%/%%/g')
grep -qF "Exec=\"$escaped\" " "$entry" || fail "Exec does not run the installed program"

"$package/install.sh" --prefix "$prefix" --uninstall >/dev/null
(cd "$package" && find bin share -type f) | while IFS= read -r file; do
    [ ! -e "$prefix/$file" ] || fail "uninstalling left $file behind"
done

bare="$work/bare"
bare_prefix="$work/bare-prefix"
mkdir -p "$bare"
cp "$package/install.sh" "$bare/install.sh"
if "$bare/install.sh" --prefix "$bare_prefix" >/dev/null 2>&1; then
    fail "installing from a folder without the program succeeded"
fi
[ ! -e "$bare_prefix" ] || fail "installing from a folder without the program wrote to the prefix"

blocked_prefix="$work/blocked"
mkdir -p "$blocked_prefix"
: >"$blocked_prefix/share"
if "$package/install.sh" --prefix "$blocked_prefix" >/dev/null 2>&1; then
    fail "installing where share is a file succeeded"
fi
left=$(find "$blocked_prefix" -mindepth 1 ! -path "$blocked_prefix/share" ! -type d)
[ -z "$left" ] || fail "a failed install left files behind: $left"

echo "Installed and uninstalled $(basename "$archive") cleanly."
