# Checks a Windows download as a player gets it. Nothing here speaks or
# opens a window.
#
# 1. The game in the folder that scripts/stage-release.ps1 made fetches the
#    original game's data, finds it, and the data has the same bytes as the
#    checkout that scripts/fetch-upstream.ps1 makes.
# 2. With -Installer: the installer installs without a question, the
#    installed game finds the same data, and the uninstaller removes the
#    program and its Start menu entry.
#
# The data goes to a folder that stands for %APPDATA%, not to yours. The
# installer check does install for the current user, then uninstalls.

param(
    # The folder scripts/stage-release.ps1 made.
    [Parameter(Mandatory)] [string] $Folder,
    # The installer made from that folder.
    [string] $Installer,
    # A new folder for what the check writes. It is removed first.
    [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) 'soundrts-release-check')
)

$ErrorActionPreference = 'Stop'

$Repo = Split-Path -Parent $PSScriptRoot
$Upstream = Join-Path $Repo 'vendor/soundrts-upstream'
$Commit = (Get-Content (Join-Path $PSScriptRoot 'upstream-commit.txt') -Raw).Trim()
$DataFolders = 'res', 'cfg', 'mods', 'single'

if (-not (Test-Path (Join-Path $Upstream 'res/rules.txt'))) {
    throw "$Upstream is missing: run scripts/fetch-upstream.ps1 first."
}
$Folder = (Resolve-Path $Folder).Path
if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
New-Item -ItemType Directory -Force $Work | Out-Null
$AppData = Join-Path $Work 'appdata'
New-Item -ItemType Directory $AppData | Out-Null
$ExpectedRoot = Join-Path $AppData "SoundRTS\data\$Commit"

# Runs a program to its end and returns what it printed. The game is a
# windowed program: it prints only to a stream it is given, and a shell does
# not wait for it unless told to.
function Invoke-Program([string] $Program, [string[]] $Arguments, [hashtable] $Environment = @{}) {
    $out = Join-Path $Work 'out.txt'
    $err = Join-Path $Work 'err.txt'
    $saved = @{}
    foreach ($name in $Environment.Keys) {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name, $Environment[$name])
    }
    try {
        $process = Start-Process -FilePath $Program -ArgumentList $Arguments -Wait -PassThru -NoNewWindow `
            -WorkingDirectory (Split-Path -Parent $Program) -RedirectStandardOutput $out -RedirectStandardError $err
    }
    finally {
        foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
    $text = (Get-Content $out -Raw) + (Get-Content $err -Raw)
    if ($process.ExitCode -ne 0) {
        throw "$Program $($Arguments -join ' ') ended with exit code $($process.ExitCode):`n$text"
    }
    $text
}

# The game's own report of what it found must name the fetched data.
function Assert-FindsTheData([string] $Game) {
    $report = Invoke-Program $Game @('--probe') @{ APPDATA = $AppData }
    Write-Output $report
    $root = ($report -split "`r?`n" | Where-Object { $_ -like 'game root: *' }) -replace '^game root: ', ''
    if ($root -ne $ExpectedRoot) {
        throw "$Game found its data in '$root', not in '$ExpectedRoot'."
    }
}

function Get-Hashes([string] $Root) {
    $hashes = @{}
    foreach ($folder in $DataFolders) {
        Get-ChildItem (Join-Path $Root $folder) -Recurse -File | ForEach-Object {
            $hashes[[IO.Path]::GetRelativePath($Root, $_.FullName)] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash
        }
    }
    $hashes
}

# 1. The folder as the zip holds it.
$game = Join-Path $Folder 'soundrts.exe'
Write-Output (Invoke-Program $game @('--fetch-data') @{ APPDATA = $AppData })
Assert-FindsTheData $game

$fetched = Get-Hashes $ExpectedRoot
$checkout = Get-Hashes $Upstream
$different = @($checkout.Keys | Where-Object { $fetched[$_] -ne $checkout[$_] }) +
    @($fetched.Keys | Where-Object { -not $checkout.ContainsKey($_) })
if ($different.Count -gt 0) {
    throw "The fetched data differs from $Upstream in $($different.Count) files, for example $($different[0])."
}
Write-Output "The fetched data is the checkout's: $($fetched.Count) files, the same bytes."

$server = Invoke-Program (Join-Path $Folder 'soundrts-server.exe') @('--help')
if ($server -notlike '*--port*') { throw "soundrts-server.exe --help does not describe --port:`n$server" }
Write-Output 'The server answers --help.'

if (-not $Installer) { return }

# 2. The installer.
$Installer = (Resolve-Path $Installer).Path
$installed = Join-Path $Work 'installed'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'sound-rst.lnk'
$log = Join-Path $Work 'install.log'
$process = Start-Process -FilePath $Installer -Wait -PassThru `
    -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$installed`"", "/LOG=`"$log`""
if ($process.ExitCode -ne 0) {
    throw "The installer ended with exit code $($process.ExitCode):`n$(Get-Content $log -Raw)"
}
try {
    $staged = Get-ChildItem $Folder -Recurse -File | ForEach-Object { [IO.Path]::GetRelativePath($Folder, $_.FullName) }
    $missing = @($staged | Where-Object { -not (Test-Path (Join-Path $installed $_)) })
    if ($missing.Count -gt 0) { throw "The installer did not install: $($missing -join ', ')." }
    if (-not (Test-Path $shortcut)) { throw "The installer made no Start menu entry at $shortcut." }
    Assert-FindsTheData (Join-Path $installed 'soundrts.exe')
    Write-Output "The installer installed $($staged.Count) files and a Start menu entry."
}
finally {
    $uninstaller = Join-Path $installed 'unins000.exe'
    if (Test-Path $uninstaller) {
        Start-Process -FilePath $uninstaller -Wait -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
        # The uninstaller finishes its work in a copy of itself.
        foreach ($i in 1..60) {
            if (-not (Test-Path $installed)) { break }
            Start-Sleep -Milliseconds 500
        }
    }
}
if (Test-Path $installed) { throw "The uninstaller left $installed." }
if (Test-Path $shortcut) { throw "The uninstaller left $shortcut." }
if (-not (Test-Path (Join-Path $ExpectedRoot 'res/rules.txt'))) {
    throw 'The uninstaller removed the fetched data, which belongs to the player.'
}
Write-Output 'The uninstaller removed the program and its Start menu entry, and left the data.'
