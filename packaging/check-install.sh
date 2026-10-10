#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/check-install.sh ARCHIVE

Extracts a release archive built by packaging/build-release.sh, installs it
with its install.sh into a temporary prefix, checks every packaged file, the
menu entry and the program, then uninstalls it and checks nothing is left.

The installed program is also checked against docs/RELEASING.md (check-binary.sh: the
libraries it links and its highest glibc symbol) and, when Xvfb is installed, started to
a first frame, killed with an edit journalled and started again to restore it, on Vulkan and on
OpenGL, for an untitled document and an opened model (check-run.sh).
CADITOR_REQUIRE_DISPLAY=1 makes a missing Xvfb a failure, as CI sets it.
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

here=$(cd "$(dirname "$0")" && pwd)
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
"$here/check-binary.sh" "$prefix/bin/caditor"
if [ -n "${CADITOR_REQUIRE_DISPLAY:-}" ] || command -v Xvfb >/dev/null 2>&1; then
    "$here/check-run.sh" "$prefix/bin/caditor"
else
    echo "check-install: Xvfb is not installed, so the program is not started to a first frame" >&2
fi

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

upgraded_prefix="$work/upgraded"
"$package/install.sh" --prefix "$upgraded_prefix" >/dev/null
before=$(cd "$upgraded_prefix" && find . -type f -exec cksum {} + | LC_ALL=C sort)
newer="$work/newer"
cp -R "$package" "$newer"
mkdir -p "$newer/share/doc/caditor/added"
echo "a file only the newer release has" >"$newer/share/doc/caditor/added/NOTES"
: >"$upgraded_prefix/share/doc/caditor/added"
if "$newer/install.sh" --prefix "$upgraded_prefix" >/dev/null 2>&1; then
    fail "an upgrade that could not copy every file succeeded"
fi
after=$(cd "$upgraded_prefix" && find . -type f ! -path ./share/doc/caditor/added -exec cksum {} + \
    | LC_ALL=C sort)
[ "$before" = "$after" ] || fail "an upgrade that failed midway changed the earlier install"
"$upgraded_prefix/bin/caditor" --version >/dev/null \
    || fail "the earlier install no longer runs after a failed upgrade"

echo "Installed and uninstalled $(basename "$archive") cleanly."
