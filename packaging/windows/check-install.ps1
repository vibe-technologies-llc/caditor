param([Parameter(Mandatory)][string]$Msi)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    Write-Error "check-install: $Message"
    exit 1
}

function Invoke-Installer([string]$Action, [string]$What) {
    $log = Join-Path ([IO.Path]::GetTempPath()) "caditor-$What.log"
    $process = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @(
        $Action, "`"$package`"", '/qn', '/norestart', '/l*v', "`"$log`""
    )
    if ($process.ExitCode -ne 0) {
        Get-Content $log -Tail 60 | Write-Output
        Fail "msiexec $Action failed with exit code $($process.ExitCode) while $What"
    }
}

$package = (Resolve-Path $Msi).Path
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"$' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$folder = Join-Path $env:LOCALAPPDATA 'Programs\caditor'
$program = Join-Path $folder 'caditor.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'caditor.lnk'
$progId = 'HKCU:\Software\Classes\caditor.model'
$extension = 'HKCU:\Software\Classes\.caditor'
$installed = @(
    'caditor.exe',
    'README.md',
    'licenses\LICENSE.txt',
    'licenses\Inter-LICENSE.txt'
)

function Assert-Installed([string]$When) {
    foreach ($file in $installed) {
        if (-not (Test-Path -PathType Leaf (Join-Path $folder $file))) {
            Fail "$file is missing from $folder $When"
        }
    }
    $reported = (& $program --version | Out-String).Trim()
    if ($reported -ne "caditor $version") {
        Fail "the installed program reports '$reported' rather than 'caditor $version' $When"
    }
    if (-not (Test-Path -PathType Leaf $shortcut)) {
        Fail "the Start menu shortcut is missing $When"
    }
    $target = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath
    if ($target -ne $program) {
        Fail "the Start menu shortcut opens '$target' rather than '$program' $When"
    }
    $associated = (Get-ItemProperty $extension).'(default)'
    if ($associated -ne 'caditor.model') {
        Fail ".caditor files open with '$associated' rather than caditor.model $When"
    }
    $command = (Get-ItemProperty "$progId\shell\open\command").'(default)'
    if ($command -ne "`"$program`" `"%1`"") {
        Fail "caditor.model opens with '$command' $When"
    }
}

if (Test-Path $folder) {
    Fail "$folder already exists, so this check cannot tell what the installer did"
}

Invoke-Installer '/i' 'installing'
Assert-Installed 'after installing'

Invoke-Installer '/i' 'installing over itself'
Assert-Installed 'after installing over itself'

Invoke-Installer '/x' 'uninstalling'
foreach ($left in $folder, $shortcut, $progId, $extension, 'HKCU:\Software\caditor') {
    if (Test-Path $left) {
        Fail "uninstalling left $left behind"
    }
}

Write-Output "check-install: $package installs, upgrades and uninstalls cleanly"
