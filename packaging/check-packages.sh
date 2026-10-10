#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/check-packages.sh [--install] ARCHIVE

Checks the .deb, .rpm and .AppImage that packaging/build-packages.sh made beside a
release archive: their checksums, their metadata, that each holds exactly the files of
the archive (plus the Debian copyright file), that the program in each runs and reports
the archive's version, and that the menu entry validates.
--install also installs the .deb with apt-get as root, runs the installed program,
removes the package and checks nothing is left (it changes the system: use it in a
disposable container).
EOF
}

fail() {
    echo "check-packages: $1" >&2
    exit 1
}

install_deb=false
case "${1:-}" in
    --install)
        install_deb=true
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
esac
if [ $# -ne 1 ]; then
    usage >&2
    exit 2
fi

for tool in ar tar xz zstd rpm rpm2cpio cpio sha256sum md5sum desktop-file-validate diff; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is needed"
done

archive=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
[ -f "$archive" ] || fail "$archive does not exist"
dist=$(dirname "$archive")
name=$(basename "$archive" .tar.zst)
label=${name#caditor-}
label=${label%-linux-*}
version=${label%-snapshot}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

zstd -dc "$archive" | tar -xf - -C "$work"
(cd "$work/$name" && find bin share -type f | sed 's|^|usr/|' | LC_ALL=C sort) >"$work/expected"
echo "usr/share/doc/caditor/copyright" >>"$work/expected"
LC_ALL=C sort -o "$work/expected" "$work/expected"
(cd "$work/$name" && find bin share -type f | LC_ALL=C sort) >"$work/expected-appimage"

for package in "$name.deb" "$name.rpm" "$name.AppImage"; do
    [ -f "$dist/$package" ] || fail "$dist/$package does not exist"
    (cd "$dist" && sha256sum -c --status "$package.sha256") || fail "$package does not match its checksum"
done

check_tree() {
    tree=$1
    expected=$2
    prefix=$3
    (cd "$tree" && find . -type f | sed 's|^\./||' | LC_ALL=C sort) >"$work/found"
    diff "$expected" "$work/found" >"$work/difference" \
        || fail "$4 holds other files than the archive: $(cat "$work/difference")"
    [ -x "$tree/${prefix}bin/caditor" ] || fail "$4's program is not executable"
    writable=$(find "$tree" -type f -perm /022)
    [ -z "$writable" ] || fail "$4 holds files others can write: $writable"
    reported=$("$tree/${prefix}bin/caditor" --version)
    [ "$reported" = "caditor $version" ] || fail "$4's program reports '$reported', not 'caditor $version'"
    desktop-file-validate "$tree/${prefix}share/applications/caditor.desktop"
    cmp -s "$work/$name/share/applications/caditor.desktop" "$tree/${prefix}share/applications/caditor.desktop" \
        || fail "$4's menu entry differs from the archive's"
}

deb="$dist/$name.deb"
members=$(ar t "$deb" | tr '\n' ' ')
[ "$members" = "debian-binary control.tar.xz data.tar.xz " ] || fail "the .deb holds $members"
[ "$(ar p "$deb" debian-binary)" = "2.0" ] || fail "the .deb is not format 2.0"
mkdir "$work/deb" "$work/deb-control"
ar p "$deb" control.tar.xz | tar -xJf - -C "$work/deb-control"
ar p "$deb" data.tar.xz | tar -xJf - -C "$work/deb"
control="$work/deb-control/control"
for field in Package Version Architecture Maintainer Installed-Size Depends Description; do
    grep -q "^$field: " "$control" || fail "the .deb's control file has no $field"
done
grep -qx "Package: caditor" "$control" || fail "the .deb is not named caditor"
grep -Eq "^Depends: libc6 \(>= 2\.[0-9]+\)," "$control" || fail "the .deb does not depend on a glibc"
grep -q "libvulkan1" "$control" || fail "the .deb does not depend on the Vulkan loader"
(cd "$work/deb" && md5sum -c --quiet "$work/deb-control/md5sums") || fail "the .deb's md5sums do not match its files"
check_tree "$work/deb" "$work/expected" usr/ "the .deb"
if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb --info "$deb" >/dev/null || fail "dpkg-deb cannot read the .deb"
    dpkg-deb --contents "$deb" >/dev/null || fail "dpkg-deb cannot list the .deb"
fi

rpm_file="$dist/$name.rpm"
rpm_query() {
    rpm -qp --dbpath "$work/rpmdb" "$@" "$rpm_file"
}
[ "$(rpm_query --queryformat '%{NAME}')" = caditor ] || fail "the .rpm is not named caditor"
case "$(rpm_query --queryformat '%{VERSION}')" in
    "$version") ;;
    *) fail "the .rpm is version $(rpm_query --queryformat '%{VERSION}'), not $version" ;;
esac
[ "$(rpm_query --queryformat '%{LICENSE}')" = "AGPL-3.0-only" ] || fail "the .rpm is not licensed AGPL-3.0-only"
rpm_query --requires >"$work/rpm-requires"
grep -q "^libc.so.6(GLIBC_2\.[0-9]*)(64bit)$" "$work/rpm-requires" || fail "the .rpm does not require a glibc"
grep -qx "vulkan-loader" "$work/rpm-requires" || fail "the .rpm does not require the Vulkan loader"
mkdir "$work/rpm"
(cd "$work/rpm" && rpm2cpio "$rpm_file" | cpio -idm --quiet)
grep -v '^usr/share/doc/caditor/copyright$' "$work/expected" >"$work/expected-rpm"
check_tree "$work/rpm" "$work/expected-rpm" usr/ "the .rpm"

appimage="$dist/$name.AppImage"
[ -x "$appimage" ] || fail "the .AppImage is not executable"
mkdir "$work/appimage"
(cd "$work/appimage" && APPIMAGE_EXTRACT_AND_RUN=1 "$appimage" --appimage-extract >/dev/null) \
    || fail "the .AppImage cannot be extracted"
extracted="$work/appimage/squashfs-root"
[ -x "$extracted/AppRun" ] || fail "the .AppImage has no AppRun"
[ -f "$extracted/caditor.desktop" ] || fail "the .AppImage has no menu entry at its root"
[ -f "$extracted/caditor.png" ] || fail "the .AppImage has no icon at its root"
[ -f "$extracted/.DirIcon" ] || fail "the .AppImage has no .DirIcon"
desktop-file-validate "$extracted/caditor.desktop"
grep -q '^TryExec=' "$extracted/caditor.desktop" && fail "the .AppImage's menu entry names a TryExec"
(cd "$extracted/usr" && find . -type f | sed 's|^\./||' | LC_ALL=C sort) >"$work/found"
diff "$work/expected-appimage" "$work/found" >"$work/difference" \
    || fail "the .AppImage holds other files than the archive: $(cat "$work/difference")"
reported=$(APPIMAGE_EXTRACT_AND_RUN=1 "$appimage" --version)
[ "$reported" = "caditor $version" ] || fail "the .AppImage reports '$reported', not 'caditor $version'"

if [ "$install_deb" = true ]; then
    [ "$(id -u)" -eq 0 ] || fail "--install needs root"
    command -v apt-get >/dev/null 2>&1 || fail "--install needs apt-get"
    apt-get install -y --no-install-recommends "$deb" >/dev/null
    installed=$(caditor --version)
    [ "$installed" = "caditor $version" ] || fail "the installed program reports '$installed', not 'caditor $version'"
    sed 's|^|/|' "$work/expected" | while IFS= read -r file; do
        [ -f "$file" ] || fail "installing the .deb did not install $file"
    done
    apt-get remove -y caditor >/dev/null
    sed 's|^|/|' "$work/expected" | while IFS= read -r file; do
        [ ! -e "$file" ] || fail "removing the .deb left $file behind"
    done
fi

echo "check-packages: the .deb, .rpm and .AppImage of $name are as the archive is"
