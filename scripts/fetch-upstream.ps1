# Fetches the original SoundRTS (Python) at a pinned commit into
# vendor/soundrts-upstream, then imports its game data (rules, maps, sounds,
# translations) into vendor/game-data, which is what the game and the tests
# read. The Python source is the reference for the port. Safe to rerun, from
# the main checkout or from a linked worktree: run it again after a change
# to crates/import/src/original_names.txt.
#
# The machine keeps one Git clone of the original, in the main checkout's
# vendor/soundrts-upstream. A linked worktree gets the files of the pinned
# commit without a .git folder, so that `git worktree remove` has no nested
# repository to delete. vendor/soundrts-upstream.commit names the commit
# that the folder holds.
#
# The upstream source code is BSD 3-clause. The license of the game data is
# not stated upstream, so the data stays out of this repository.

$ErrorActionPreference = 'Stop'

$Url = 'https://github.com/soundmud/soundrts.git'
# The game also downloads the data of this commit when it has none.
$Commit = (Get-Content -Raw (Join-Path $PSScriptRoot 'upstream-commit.txt')).Trim()
$Repo = Split-Path -Parent $PSScriptRoot
$Target = Join-Path $Repo 'vendor/soundrts-upstream'
$Stamp = "$Target.commit"

function Invoke-Git {
    git @args
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed with exit code $LASTEXITCODE" }
}

# The main checkout is the parent of the common Git folder, which every
# worktree of the repository shares.
$common = "$(git -C $Repo rev-parse --path-format=absolute --git-common-dir)".Trim()
if ($LASTEXITCODE -ne 0) { throw "$Repo is not a Git checkout." }
$Main = Split-Path -Parent $common
$Clone = Join-Path $Main 'vendor/soundrts-upstream'
$InWorktree = (Resolve-Path $Repo).Path -ne (Resolve-Path $Main).Path

if (-not (Test-Path (Join-Path $Clone '.git'))) {
    New-Item -ItemType Directory -Force $Clone | Out-Null
    Invoke-Git -C $Clone init --quiet
    Invoke-Git -C $Clone remote add origin $Url
}

# The files must be the same bytes on every machine, whatever Git's line-end
# setting is there: tests compare against recorded hashes of what the game
# computes from them.
Invoke-Git -C $Clone config core.autocrlf false

if ($InWorktree) {
    # The main checkout's branch may pin another commit, so the clone's
    # checkout stays as it is. The worktree needs only the commit's objects.
    git -C $Clone cat-file -e "$Commit^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) {
        Invoke-Git -C $Clone fetch --quiet --depth 1 origin $Commit
    }

    $have = if (Test-Path $Stamp) { (Get-Content -Raw $Stamp).Trim() } else { '' }
    if ($have -ne $Commit -or (Test-Path (Join-Path $Target '.git'))) {
        # Built beside the target and moved into place, so that a run that
        # dies halfway leaves no half-written folder behind a stamp.
        $partial = "$Target.partial"
        $archive = "$Target.partial.tar"
        foreach ($path in @($Stamp, $partial, $archive, $Target)) {
            if (Test-Path $path) { Remove-Item -Recurse -Force $path }
        }
        New-Item -ItemType Directory -Force $partial | Out-Null
        Invoke-Git -C $Clone archive --format=tar --output=$archive $Commit
        # A relative name: GNU tar reads "C:" in a path as a remote host.
        Push-Location $partial
        try { tar -xf "../$(Split-Path -Leaf $archive)" } finally { Pop-Location }
        if ($LASTEXITCODE -ne 0) { throw "tar could not unpack $archive (exit code $LASTEXITCODE)." }
        Remove-Item -Force $archive
        Move-Item $partial $Target
    }
} else {
    $head = git -C $Clone rev-parse --verify --quiet HEAD
    if ($head -ne $Commit) {
        Invoke-Git -C $Clone fetch --quiet --depth 1 origin $Commit
        Invoke-Git -C $Clone -c advice.detachedHead=false checkout --quiet --force FETCH_HEAD
    }
}

if (-not (Test-Path (Join-Path $Target 'res/rules.txt'))) {
    throw "The folder at $Target is incomplete: res/rules.txt is missing."
}
Set-Content -NoNewline -Path $Stamp -Value $Commit

Write-Output "SoundRTS upstream is ready at $Target ($Commit)."

# The game reads only imported data: every number of the original becomes a
# name, through crates/import/src/original_names.txt.
$Data = Join-Path (Split-Path -Parent $PSScriptRoot) 'vendor/game-data'
$Manifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Cargo.toml'
cargo run --quiet --manifest-path $Manifest -p soundrts-import -- $Target $Data
if ($LASTEXITCODE -ne 0) { throw "The import of $Target failed with exit code $LASTEXITCODE." }
Write-Output "The imported game data is ready at $Data."
