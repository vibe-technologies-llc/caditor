#!/bin/sh
set -eu

usage() {
    cat <<EOF
Usage: packaging/build-release.sh [--snapshot]

Builds the release archive of the checked-out commit into target/dist:
caditor-<version>-linux-<arch>.tar.zst with its .sha256, and release-notes.md
taken from the version's section of CHANGELOG.md.

A release needs a clean tree, HEAD tagged v<version> and a dated
"## [<version>] - YYYY-MM-DD" section in CHANGELOG.md.
--snapshot skips those checks, names the archive <version>-snapshot and takes
the notes from the Unreleased section.
EOF
}

fail() {
    echo "build-release: $1" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is needed ($2)"
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
need cargo-about "cargo install --locked cargo-about --features cli"
need desktop-file-validate "desktop-file-utils"
need appstreamcli "appstream"
need zstd "zstd"
need sha256sum "coreutils"
need git "git"

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "no workspace version in Cargo.toml"
version_pattern=$(printf '%s' "$version" | sed 's/\./\\./g')
release_date=$(sed -n "s/^## \[$version_pattern\] - \([0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}\)\$/\1/p" CHANGELOG.md)

if [ "$snapshot" = true ]; then
    label="$version-snapshot"
    notes_heading="## [Unreleased]"
else
    [ -z "$(git status --porcelain)" ] || fail "the working tree has uncommitted changes"
    tag=$(git describe --exact-match --tags HEAD 2>/dev/null || true)
    [ "$tag" = "v$version" ] || fail "HEAD must be tagged v$version, not '${tag:-untagged}'"
    [ -n "$release_date" ] || fail "CHANGELOG.md has no '## [$version] - YYYY-MM-DD' section"
    label="$version"
    notes_heading="## [$version]"
fi

arch=$(uname -m)
target_dir=${CARGO_TARGET_DIR:-$root/target}
dist="$target_dir/dist"
name="caditor-$label-linux-$arch"
stage="$dist/$name"
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
export SOURCE_DATE_EPOCH

cargo build --release --locked -p caditor

rm -rf "$stage" "$dist/$name.tar.zst" "$dist/$name.tar.zst.sha256" "$dist/release-notes.md"
mkdir -p "$stage"

install -D -m 755 "$target_dir/release/caditor" "$stage/bin/caditor"
install -D -m 644 packaging/caditor.desktop "$stage/share/applications/caditor.desktop"
install -D -m 644 packaging/caditor.svg "$stage/share/icons/hicolor/scalable/apps/caditor.svg"
install -D -m 644 packaging/caditor-mime.xml "$stage/share/mime/packages/caditor.xml"
install -D -m 644 LICENSE "$stage/share/licenses/caditor/LICENSE"
install -D -m 644 crates/caditor/assets/fonts/Inter-LICENSE.txt \
    "$stage/share/licenses/caditor/Inter-LICENSE.txt"
install -D -m 644 README.md "$stage/share/doc/caditor/README.md"
install -D -m 644 CHANGELOG.md "$stage/share/doc/caditor/CHANGELOG.md"
install -m 755 packaging/install.sh "$stage/install.sh"
install -m 644 packaging/INSTALL.md "$stage/INSTALL.md"

releases=$(sed -n 's/^## \[\([0-9][0-9A-Za-z.+-]*\)\] - \([0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}\)$/    <release version="\1" date="\2"\/>/p' CHANGELOG.md)
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

cargo about generate --locked -c packaging/about.toml \
    -o "$stage/share/licenses/caditor/THIRD-PARTY-LICENSES.html" packaging/about.hbs

desktop-file-validate "$stage/share/applications/caditor.desktop"
metainfo_report=$(appstreamcli validate --no-net "$stage/share/metainfo/caditor.metainfo.xml" || true)
if printf '%s\n' "$metainfo_report" | grep -q '^E:'; then
    printf '%s\n' "$metainfo_report" >&2
    fail "the metainfo file has errors"
fi

reported=$("$stage/bin/caditor" --version)
[ "$reported" = "caditor $version" ] || fail "the binary reports '$reported', not 'caditor $version'"

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH" \
    -C "$dist" -cf - "$name" | zstd -q -19 -T0 -o "$dist/$name.tar.zst"
(cd "$dist" && sha256sum "$name.tar.zst" >"$name.tar.zst.sha256")

awk -v heading="$notes_heading" '
    index($0, heading) == 1 { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
' CHANGELOG.md | sed -e '/./,$!d' >"$dist/release-notes.md"

echo "$dist/$name.tar.zst"
