#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/build-packages.sh ARCHIVE

Builds the Linux packages from a release archive made by packaging/build-release.sh,
beside it: caditor-<label>-linux-<arch>.deb, .rpm and .AppImage, each with a .sha256.
They hold the archive's own program, menu entry, icons, metainfo and licences, so
they are the build the archive is.

Needs ar and objdump (binutils), xz, tar, zstd, rpmbuild (rpm), mksquashfs
(squashfs-tools), curl and sha256sum. The AppImage runtime is downloaded once, from
the version pinned in .github/versions.env, and checked against its SHA-256; setting
APPIMAGE_RUNTIME to a runtime file uses that instead.
EOF
}

fail() {
    echo "build-packages: $1" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is needed ($2)"
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

need ar "binutils"
need objdump "binutils"
need xz "xz-utils"
need tar "tar"
need zstd "zstd"
need rpmbuild "rpm"
need mksquashfs "squashfs-tools"
need curl "curl"
need sha256sum "coreutils"
need md5sum "coreutils"
need git "git"

root=$(cd "$(dirname "$0")/.." && pwd)
archive=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
[ -f "$archive" ] || fail "$archive does not exist"

dist=$(dirname "$archive")
base=$(basename "$archive" .tar.zst)
case "$base" in
    caditor-*-linux-*) ;;
    *) fail "$base is not named caditor-<label>-linux-<arch>" ;;
esac
label=${base#caditor-}
label=${label%-linux-*}
arch=${base##*-linux-}
name="caditor-$label-linux-$arch"

version=${label%-snapshot}
if [ "$version" = "$label" ]; then
    deb_version=$version
    rpm_release=1
else
    deb_version="$version~snapshot"
    rpm_release=0.snapshot
fi

case "$arch" in
    x86_64) deb_arch=amd64 ;;
    aarch64) deb_arch=arm64 ;;
    *) fail "no package architecture is known for $arch" ;;
esac

cd "$root"
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
export SOURCE_DATE_EPOCH
umask 022

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

zstd -dc "$archive" | tar -xf - -C "$work"
unpacked="$work/$name"
[ -x "$unpacked/bin/caditor" ] || fail "the archive holds no $name/bin/caditor"

glibc=$(objdump -T "$unpacked/bin/caditor" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1)
[ -n "$glibc" ] || fail "the program names no glibc version"

summary="Parametric CAD"
description_lines="caditor designs parts from constrained 2D sketches and features that stay editable:
change a dimension or an early sketch and everything after it follows."

lay_out() {
    target=$1
    (cd "$unpacked" && find bin share -type f | LC_ALL=C sort) | while IFS= read -r file; do
        case "$file" in
            bin/*) mode=755 ;;
            *) mode=644 ;;
        esac
        install -D -m "$mode" "$unpacked/$file" "$target/usr/$file"
    done
}

finish() {
    (cd "$dist" && sha256sum "$1" >"$1.sha256")
    echo "$dist/$1"
}

build_deb() {
    deb="$work/deb"
    mkdir -p "$deb/control"
    lay_out "$deb/data"
    install -D -m 644 "$unpacked/share/licenses/caditor/LICENSE" "$deb/data/usr/share/doc/caditor/copyright"

    (cd "$deb/data" && find usr -type f | LC_ALL=C sort | while IFS= read -r file; do md5sum "$file"; done) \
        >"$deb/control/md5sums"
    size=$(du -sk --apparent-size "$deb/data" | cut -f 1)

    {
        echo "Package: caditor"
        echo "Version: $deb_version"
        echo "Architecture: $deb_arch"
        echo "Maintainer: caditor"
        echo "Installed-Size: $size"
        echo "Section: graphics"
        echo "Priority: optional"
        echo "Depends: libc6 (>= $glibc), libgcc-s1, libxkbcommon0, libvulkan1, hicolor-icon-theme"
        echo "Recommends: xdg-desktop-portal | zenity, libwayland-client0, libwayland-egl1, libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxi6, libxkbcommon-x11-0, libegl1"
        echo "Suggests: mesa-vulkan-drivers"
        echo "Description: $summary"
        printf '%s\n' "$description_lines" | sed 's/^/ /'
    } >"$deb/control/control"

    printf '2.0\n' >"$deb/debian-binary"
    tar_deterministic "$deb/control" | xz -9 -T1 >"$deb/control.tar.xz"
    tar_deterministic "$deb/data" | xz -9 -T1 >"$deb/data.tar.xz"

    out="$name.deb"
    rm -f "$dist/$out" "$dist/$out.sha256"
    (cd "$deb" && ar rcSD "$dist/$out" debian-binary control.tar.xz data.tar.xz)
    finish "$out"
}

tar_deterministic() {
    tar --sort=name --owner=0 --group=0 --numeric-owner --format=gnu \
        --mtime="@$SOURCE_DATE_EPOCH" -C "$1" -cf - .
}

build_rpm() {
    rpm="$work/rpm"
    mkdir -p "$rpm/BUILD" "$rpm/RPMS" "$rpm/SOURCES" "$rpm/SPECS" "$rpm/SRPMS"
    lay_out "$rpm/stage"

    (cd "$rpm/stage" && find usr -type f | LC_ALL=C sort | sed 's|^|/|') >"$rpm/files"
    {
        echo "%dir /usr/share/doc/caditor"
        echo "%dir /usr/share/licenses/caditor"
        cat "$rpm/files"
    } >"$rpm/filelist"

    cat >"$rpm/SPECS/caditor.spec" <<EOF
%global debug_package %{nil}
%global _build_id_links none
%global __os_install_post %{nil}
%global _binary_payload w9.gzdio

Name: caditor
Version: $version
Release: $rpm_release
Summary: $summary
License: AGPL-3.0-only
Requires: libc.so.6(GLIBC_$glibc)(64bit)
Requires: libgcc
Requires: libxkbcommon
Requires: vulkan-loader
Requires: hicolor-icon-theme
Recommends: xdg-desktop-portal
Recommends: libwayland-client
Recommends: libwayland-egl
Recommends: libX11
Recommends: libX11-xcb
Recommends: libxcb
Recommends: libXcursor
Recommends: libXi
Recommends: libxkbcommon-x11
Recommends: libglvnd-egl

%description
$description_lines

%install
mkdir -p %{buildroot}
cp -a $rpm/stage/usr %{buildroot}/usr

%files -f $rpm/filelist
%defattr(-,root,root,-)
EOF

    rpmbuild -bb --quiet \
        --define "_topdir $rpm" \
        --define "_rpmdir $rpm/RPMS" \
        --define "_dbpath $rpm/db" \
        --define "source_date_epoch_from_changelog 0" \
        --define "use_source_date_epoch_as_buildtime 1" \
        --define "clamp_mtime_to_source_date_epoch 1" \
        --define "build_mtime_policy clamp_to_source_date_epoch" \
        "$rpm/SPECS/caditor.spec"

    built=$(find "$rpm/RPMS" -name '*.rpm')
    [ -n "$built" ] || fail "rpmbuild produced no package"
    out="$name.rpm"
    rm -f "$dist/$out" "$dist/$out.sha256"
    cp "$built" "$dist/$out"
    finish "$out"
}

appimage_runtime() {
    if [ -n "${APPIMAGE_RUNTIME:-}" ]; then
        [ -f "$APPIMAGE_RUNTIME" ] || fail "$APPIMAGE_RUNTIME does not exist"
        echo "$APPIMAGE_RUNTIME"
        return
    fi
    [ "$arch" = x86_64 ] || fail "the pinned AppImage runtime is for x86_64; set APPIMAGE_RUNTIME for $arch"
    if [ -z "${APPIMAGE_RUNTIME_VERSION:-}" ] || [ -z "${APPIMAGE_RUNTIME_SHA256:-}" ]; then
        . "$root/.github/versions.env"
    fi
    cache="${CARGO_TARGET_DIR:-$root/target}/appimage-runtime"
    runtime="$cache/runtime-$arch-$APPIMAGE_RUNTIME_VERSION"
    if ! { [ -f "$runtime" ] && echo "$APPIMAGE_RUNTIME_SHA256  $runtime" | sha256sum -c --status; }; then
        mkdir -p "$cache"
        curl --proto '=https' --tlsv1.2 -sSfL -o "$runtime.part" \
            "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$arch"
        echo "$APPIMAGE_RUNTIME_SHA256  $runtime.part" | sha256sum -c --status \
            || fail "the downloaded AppImage runtime does not match its pinned SHA-256"
        mv "$runtime.part" "$runtime"
    fi
    echo "$runtime"
}

build_appimage() {
    runtime=$(appimage_runtime)
    appdir="$work/appimage/AppDir"
    lay_out "$appdir"

    cat >"$appdir/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/caditor" "$@"
EOF
    chmod 755 "$appdir/AppRun"
    grep -v '^TryExec=' "$unpacked/share/applications/caditor.desktop" >"$appdir/caditor.desktop"
    chmod 644 "$appdir/caditor.desktop"
    install -m 644 "$unpacked/share/icons/hicolor/256x256/apps/caditor.png" "$appdir/caditor.png"
    ln -s caditor.png "$appdir/.DirIcon"

    squashfs="$work/appimage/caditor.squashfs"
    mksquashfs "$appdir" "$squashfs" -noappend -quiet -no-progress -root-owned -no-xattrs \
        -comp zstd -Xcompression-level 19 -b 256K

    out="$name.AppImage"
    rm -f "$dist/$out" "$dist/$out.sha256"
    cat "$runtime" "$squashfs" >"$dist/$out"
    chmod 755 "$dist/$out"
    finish "$out"
}

build_deb
build_rpm
build_appimage
