#!/bin/sh
set -eu

usage() {
    cat <<USAGE
Usage: .github/bump-pins.sh

Raises every pin CI and the release build use to its latest published version:
the toolchains, rustup, cargo-deny, cargo-fuzz and WiX 5 with
their SHA-256 sums and rust-formatter's revision in .github/versions.env, and each
action in .github/workflows and .github/actions to the commit of its latest
release. Review the diff and let CI run before committing it.
USAGE
}

fail() {
    echo "bump-pins: $1" >&2
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

for tool in curl git sha256sum sed awk; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is needed"
done

root=$(cd "$(dirname "$0")/.." && pwd)
versions="$root/.github/versions.env"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

fetch() {
    curl --proto '=https' --tlsv1.2 -sSfL "$1"
}

latest_release() {
    fetch "https://api.github.com/repos/$1/releases/latest" |
        sed -n 's/^ *"tag_name": *"\([^"]*\)".*/\1/p'
}

tag_commit() {
    refs=$(git ls-remote "https://github.com/$1" "refs/tags/$2" "refs/tags/$2^{}")
    peeled=$(printf '%s\n' "$refs" | awk '$2 ~ /\^\{\}$/ { print $1 }')
    if [ -n "$peeled" ]; then
        echo "$peeled"
    else
        printf '%s\n' "$refs" | awk 'NR == 1 { print $1 }'
    fi
}

sha256_of() {
    fetch "$1" >"$scratch/download"
    sha256sum "$scratch/download" | awk '{ print $1 }'
}

set_pin() {
    grep -q "^$1=" "$versions" || fail "$1 is not in $versions"
    sed -i "s|^$1=.*|$1=$2|" "$versions"
    echo "$1=$2"
}

stable=$(fetch https://static.rust-lang.org/dist/channel-rust-stable.toml |
    awk '/^\[pkg\.rust\]$/ { found = 1; next } found && !done && /^version = / { print $3; done = 1 }' |
    tr -d '"')
[ -n "$stable" ] || fail "could not read the latest stable Rust"
set_pin RUST_TOOLCHAIN "$stable"

nightly_date=$(fetch https://static.rust-lang.org/dist/channel-rust-nightly.toml |
    sed -n 's/^date = "\(.*\)"$/\1/p')
[ -n "$nightly_date" ] || fail "could not read the latest nightly"
set_pin FUZZ_TOOLCHAIN "nightly-$nightly_date"
set_pin FORMAT_TOOLCHAIN "nightly-$nightly_date"

rustup=$(fetch https://static.rust-lang.org/rustup/release-stable.toml |
    sed -n "s/^version = '\(.*\)'$/\1/p")
[ -n "$rustup" ] || fail "could not read the latest rustup"
rustup_sum=$(sha256_of "https://static.rust-lang.org/rustup/archive/$rustup/x86_64-unknown-linux-gnu/rustup-init")
set_pin RUSTUP_VERSION "$rustup"
set_pin RUSTUP_SHA256 "$rustup_sum"
rustup_windows_sum=$(sha256_of "https://static.rust-lang.org/rustup/archive/$rustup/x86_64-pc-windows-msvc/rustup-init.exe")
set_pin RUSTUP_WINDOWS_SHA256 "$rustup_windows_sum"

formatter=$(git ls-remote https://github.com/vibe-technologies-llc/rust-formatter HEAD | awk '{ print $1 }')
[ -n "$formatter" ] || fail "could not read rust-formatter's latest revision"
set_pin RUST_FORMATTER_REV "$formatter"

bump_binary() {
    variable=$1
    repository=$2
    name=$3
    version=$(latest_release "$repository")
    [ -n "$version" ] || fail "could not read the latest release of $repository"
    archive="$name-$version-x86_64-unknown-linux-musl.tar.gz"
    sum=$(sha256_of "https://github.com/$repository/releases/download/$version/$archive")
    set_pin "${variable}_VERSION" "$version"
    set_pin "${variable}_SHA256" "$sum"
}

bump_binary CARGO_DENY EmbarkStudios/cargo-deny cargo-deny
bump_binary CARGO_FUZZ rust-fuzz/cargo-fuzz cargo-fuzz

wix=$(fetch https://api.nuget.org/v3-flatcontainer/wix/index.json |
    tr ',' '\n' | sed -n 's/^.*"\(5\.[0-9]*\.[0-9]*\)".*$/\1/p' | tail -n 1)
[ -n "$wix" ] || fail "could not read the latest WiX 5"
set_pin WIX_VERSION "$wix"
set_pin WIX_SHA256 "$(sha256_of "https://api.nuget.org/v3-flatcontainer/wix/$wix/wix.$wix.nupkg")"

files=$(find "$root/.github/workflows" "$root/.github/actions" -name '*.yml')
actions=$(cat $files | sed -n 's/^ *uses: \([A-Za-z0-9_.-]*\/[A-Za-z0-9_.-]*\)@[0-9a-f]\{40\}$/\1/p' | sort -u)
for action in $actions; do
    tag=$(latest_release "$action")
    [ -n "$tag" ] || fail "could not read the latest release of $action"
    commit=$(tag_commit "$action" "$tag")
    [ -n "$commit" ] || fail "could not find the commit of $action $tag"
    for file in $files; do
        sed -i \
            -e "s|uses: $action@[0-9a-f]\{40\}$|uses: $action@$commit|" \
            -e "s|($action v[^)]*)|($action $tag)|" \
            "$file"
    done
    echo "$action $tag $commit"
done
