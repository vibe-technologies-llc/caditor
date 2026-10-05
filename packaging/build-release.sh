#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/build-release.sh [--snapshot]

Builds the release archive of the checked-out commit into target/dist:
caditor-<version>-linux-<arch>.tar.zst with its .sha256. The metainfo lists the
release, dated by the tagged commit.

A release needs a clean tree and HEAD tagged v<version>.
--snapshot skips those checks, names the archive <version>-snapshot and lists
no release in the metainfo.
EOF
}

fail() {
    echo "build-release: $1" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is needed ($2)"
}

accepted_metainfo_findings="cid-desktopapp-is-not-rdns url-homepage-missing developer-info-missing"

check_metainfo() {
    status=0
    report=$(appstreamcli validate --no-net --no-color "$1" 2>&1) || status=$?
    unexpected=$(printf '%s\n' "$report" | awk -v accepted="$accepted_metainfo_findings" '
        BEGIN { split(accepted, list, " "); for (i in list) allowed[list[i]] = 1 }
        /^[EWIP]: / { if (!($3 in allowed)) print }
    ')
    findings=$(printf '%s\n' "$report" | grep -c '^[EWIP]: ' || true)
    if [ -n "$unexpected" ]; then
        printf '%s\n' "$report" >&2
        fail "the metainfo file has findings beyond the accepted ones: $unexpected"
    fi
    if [ "$status" -ne 0 ] && [ "$findings" -eq 0 ]; then
        printf '%s\n' "$report" >&2
        fail "appstreamcli failed (exit $status) without reporting a finding"
    fi
}

snapshot=false
case "${1:-}" in
    --snapshot) snapshot=true ;;
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

need cargo "the Rust toolchain"
need desktop-file-validate "desktop-file-utils"
need appstreamcli "appstream"
need zstd "zstd"
need sha256sum "coreutils"
need git "git"

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "no workspace version in Cargo.toml"

if [ "$snapshot" = true ]; then
    label="$version-snapshot"
else
    [ -z "$(git status --porcelain)" ] || fail "the working tree has uncommitted changes"
    tag=$(git describe --exact-match --tags HEAD 2>/dev/null || true)
    [ "$tag" = "v$version" ] || fail "HEAD must be tagged v$version, not '${tag:-untagged}'"
    label="$version"
fi

arch=$(uname -m)
target_dir=${CARGO_TARGET_DIR:-$root/target}
dist="$target_dir/dist"
name="caditor-$label-linux-$arch"
stage="$dist/$name"
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
export SOURCE_DATE_EPOCH

cargo build --release --locked -p caditor

rm -rf "$stage" "$dist/$name.tar.zst" "$dist/$name.tar.zst.sha256"
mkdir -p "$stage"

install -D -m 755 "$target_dir/release/caditor" "$stage/bin/caditor"
install -D -m 644 packaging/caditor.desktop "$stage/share/applications/caditor.desktop"
install -D -m 644 packaging/caditor.svg "$stage/share/icons/hicolor/scalable/apps/caditor.svg"
for icon in packaging/icons/caditor-*.png; do
    size=${icon##*/caditor-}
    size=${size%.png}
    install -D -m 644 "$icon" "$stage/share/icons/hicolor/${size}x$size/apps/caditor.png"
done
install -D -m 644 packaging/caditor-mime.xml "$stage/share/mime/packages/caditor.xml"
install -D -m 644 LICENSE "$stage/share/licenses/caditor/LICENSE"
install -D -m 644 crates/caditor/assets/fonts/Inter-LICENSE.txt \
    "$stage/share/licenses/caditor/Inter-LICENSE.txt"
install -D -m 644 README.md "$stage/share/doc/caditor/README.md"
install -m 755 packaging/install.sh "$stage/install.sh"
install -m 644 packaging/INSTALL.md "$stage/INSTALL.md"

releases=""
if [ "$snapshot" = false ]; then
    release_date=$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%d)
    releases="    <release version=\"$version\" date=\"$release_date\"/>"
fi
mkdir -p "$stage/share/metainfo"
RELEASES="$releases" awk '
    /^  <releases\/>$/ && ENVIRON["RELEASES"] != "" {
        print "  <releases>"
        print ENVIRON["RELEASES"]
        print "  </releases>"
        next
    }
    { print }
' packaging/caditor.metainfo.xml >"$stage/share/metainfo/caditor.metainfo.xml"
chmod 644 "$stage/share/metainfo/caditor.metainfo.xml"

desktop-file-validate "$stage/share/applications/caditor.desktop"
check_metainfo "$stage/share/metainfo/caditor.metainfo.xml"

reported=$("$stage/bin/caditor" --version)
[ "$reported" = "caditor $version" ] || fail "the binary reports '$reported', not 'caditor $version'"

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH" \
    -C "$dist" -cf - "$name" | zstd -q -19 -T0 -o "$dist/$name.tar.zst"
(cd "$dist" && sha256sum "$name.tar.zst" >"$name.tar.zst.sha256")

echo "$dist/$name.tar.zst"
