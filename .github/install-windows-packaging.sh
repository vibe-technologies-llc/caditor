#!/bin/bash
set -eu

[ -n "${WIX_VERSION:-}" ] && [ -n "${WIX_SHA256:-}" ] || {
    echo "Load .github/versions.env before installing the packaging tools" >&2
    exit 1
}

directory=$(mktemp -d)
trap 'rm -rf "$directory"' EXIT

fetch() {
    curl --proto '=https' --tlsv1.2 -sSfL -o "$1" "$2"
}

wix="$directory/wix.$WIX_VERSION.nupkg"
fetch "$wix" "https://api.nuget.org/v3-flatcontainer/wix/$WIX_VERSION/wix.$WIX_VERSION.nupkg"
echo "$WIX_SHA256  $wix" | sha256sum -c -
cat >"$directory/nuget.config" <<CONFIG
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="checked" value="$(cygpath -w "$directory")" />
  </packageSources>
</configuration>
CONFIG
dotnet tool install --global wix --version "$WIX_VERSION" \
    --configfile "$(cygpath -w "$directory/nuget.config")"
echo "$USERPROFILE\\.dotnet\\tools" >>"$GITHUB_PATH"
