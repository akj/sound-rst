# Fetches the original SoundRTS (Python) at a pinned commit into
# vendor/soundrts-upstream. The Rust port reads the game data (rules, maps,
# sounds, translations) from that checkout, and the Python source is the
# reference for the port. Safe to rerun.
#
# The upstream source code is BSD 3-clause. The license of the game data is
# not stated upstream, so the data stays out of this repository.

$ErrorActionPreference = 'Stop'

$Url = 'https://github.com/soundmud/soundrts.git'
$Commit = 'f3bd98384fb991e17b55e2f40bf31bf9e1f14989'
$Target = Join-Path (Split-Path -Parent $PSScriptRoot) 'vendor/soundrts-upstream'

function Invoke-Git {
    git @args
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed with exit code $LASTEXITCODE" }
}

if (-not (Test-Path (Join-Path $Target '.git'))) {
    New-Item -ItemType Directory -Force $Target | Out-Null
    Invoke-Git -C $Target init --quiet
    Invoke-Git -C $Target remote add origin $Url
}

# The files must be the same bytes on every machine, whatever Git's line-end
# setting is there: tests compare against recorded hashes of what the game
# computes from them.
Invoke-Git -C $Target config core.autocrlf false

$head = git -C $Target rev-parse --verify --quiet HEAD
if ($head -ne $Commit) {
    Invoke-Git -C $Target fetch --quiet --depth 1 origin $Commit
    Invoke-Git -C $Target -c advice.detachedHead=false checkout --quiet --force FETCH_HEAD
}

if (-not (Test-Path (Join-Path $Target 'res/rules.txt'))) {
    throw "The checkout at $Target is incomplete: res/rules.txt is missing."
}

Write-Output "SoundRTS upstream is ready at $Target ($Commit)."
