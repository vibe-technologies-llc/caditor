param([switch]$Snapshot)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    Write-Error "build-release: $Message"
    exit 1
}

function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "$Program $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location $root

foreach ($tool in 'cargo', 'git', 'wix') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Fail "$tool is needed"
    }
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"$' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if ($Snapshot) {
    $label = "$version-snapshot"
} else {
    if (git status --porcelain) {
        Fail 'the checkout has changes; a release is built from a clean tree'
    }
    $tags = @(git tag --points-at HEAD)
    if ($tags -notcontains "v$version") {
        Fail "HEAD is not tagged v$version"
    }
    $label = $version
}

$name = "caditor-$label-windows-x86_64"
$targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$dist = Join-Path $targetDir 'dist'
$stage = Join-Path $dist $name
$msi = Join-Path $dist "$name.msi"

Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $stage, $msi, "$msi.sha256"
New-Item -ItemType Directory -Force (Join-Path $stage 'licenses') | Out-Null

Invoke-Checked cargo @('build', '--release', '--locked', '-p', 'caditor')
Copy-Item (Join-Path $targetDir 'release/caditor.exe') $stage
Copy-Item README.md $stage
Copy-Item LICENSE (Join-Path $stage 'licenses/LICENSE.txt')
Copy-Item crates/caditor/assets/fonts/Inter-LICENSE.txt (Join-Path $stage 'licenses')

$reported = (& (Join-Path $stage 'caditor.exe') --version | Out-String).Trim()
if ($reported -ne "caditor $version") {
    Fail "the program reports '$reported' rather than 'caditor $version'"
}

Invoke-Checked wix @(
    'build', 'packaging/windows/caditor.wxs',
    '-arch', 'x64',
    '-d', "Version=$version",
    '-d', "Stage=$stage",
    '-o', $msi
)

$sum = (Get-FileHash -Algorithm SHA256 $msi).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$msi.sha256", "$sum  $name.msi`n")
Write-Output $msi
