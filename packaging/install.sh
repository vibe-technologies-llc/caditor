#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix="$HOME/.local"
action=install

usage() {
    cat <<EOF
Usage: ./install.sh [--prefix DIR] [--uninstall]

Installs caditor into DIR, by default ~/.local: the program in DIR/bin, and its
menu entry, icon, file type, documentation and licences in DIR/share.
--uninstall removes exactly those files again.

Installing for every user needs root:  sudo ./install.sh --prefix /usr/local
EOF
}

fail() {
    echo "install.sh: $1" >&2
    exit 2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || fail "--prefix needs a directory"
            prefix=$2
            shift 2
            ;;
        --prefix=*)
            prefix=${1#--prefix=}
            shift
            ;;
        --uninstall)
            action=uninstall
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done

case "$prefix" in
    /*) ;;
    *) fail "the prefix must be an absolute path, not $prefix" ;;
esac
case "$prefix" in
    *[\"\`\$\\]*) fail "the prefix cannot contain quotes, backslashes or \$" ;;
esac
prefix=${prefix%/}

packaged_files() {
    (cd "$here" && find bin share -type f | LC_ALL=C sort)
}

desktop_entry="share/applications/caditor.desktop"

menu_entry() {
    program="$prefix/bin/caditor"
    EXEC_PROGRAM=$(printf '%s' "$program" | sed 's/%/%%/g') TRY_PROGRAM="$program" awk '
        /^Exec=caditor / {
            sub(/^Exec=caditor /, "")
            print "Exec=\"" ENVIRON["EXEC_PROGRAM"] "\" " $0
            next
        }
        /^TryExec=caditor$/ { print "TryExec=" ENVIRON["TRY_PROGRAM"]; next }
        { print }
    ' "$here/$desktop_entry"
}

roll_back() {
    [ -s "$work/installed" ] || return 0
    while IFS= read -r file; do
        rm -f "$prefix/$file"
    done <"$work/installed"
    echo "install.sh: the install failed, so the files it had copied were removed again." >&2
}

install_files() {
    packaged_files >"$work/packaged"
    : >"$work/installed"
    while IFS= read -r file; do
        case "$file" in
            bin/*) mode=755 ;;
            *) mode=644 ;;
        esac
        source="$here/$file"
        if [ "$file" = "$desktop_entry" ]; then
            source="$work/caditor.desktop"
            menu_entry >"$source"
        fi
        printf '%s\n' "$file" >>"$work/installed"
        install -D -m "$mode" "$source" "$prefix/$file"
    done <"$work/packaged"
}

uninstall_files() {
    packaged_files | while IFS= read -r file; do
        rm -f "$prefix/$file"
    done
    rmdir "$prefix/share/doc/caditor" "$prefix/share/licenses/caditor" 2>/dev/null || true
}

refresh_caches() {
    if command -v update-mime-database >/dev/null 2>&1 && [ -d "$prefix/share/mime" ]; then
        update-mime-database "$prefix/share/mime" || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1 && [ -d "$prefix/share/applications" ]; then
        update-desktop-database -q "$prefix/share/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 \
        && [ -f "$prefix/share/icons/hicolor/icon-theme.cache" ]; then
        gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
    fi
}

if [ "$action" = uninstall ]; then
    uninstall_files
    refresh_caches
    echo "Removed caditor from $prefix."
    echo "Models, preferences and recovery journals are kept."
    exit 0
fi

work=$(mktemp -d)
finished=false
finish() {
    [ "$finished" = true ] || roll_back
    rm -rf "$work"
}
trap finish EXIT
trap 'exit 1' HUP INT TERM

install_files
finished=true
refresh_caches
echo "Installed caditor $("$prefix/bin/caditor" --version | cut -d' ' -f2) into $prefix."
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "$prefix/bin is not on your PATH; the menu entry works, or run $prefix/bin/caditor." ;;
esac
