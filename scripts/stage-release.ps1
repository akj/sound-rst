# Gathers the files of the Windows download into dist/<Name>: the game and
# its .pdb, the server, the DLLs the game loads, and the licenses. The zip and the
# installer are both made from this folder. Safe to rerun.
#
# Build first:
#   pwsh scripts/fetch-nvda-controller.ps1
#   cargo build --release --locked -p soundrts -p soundrts-server
#
# The original game's data is not in the download: the game fetches it the
# first time it starts.

param(
    # The folder's name, for example soundrts-0.1.0-windows-x64.
    [Parameter(Mandatory)] [string] $Name,
    # Where Cargo put the programs.
    [string] $Target = 'target/release'
)

$ErrorActionPreference = 'Stop'

$Repo = Split-Path -Parent $PSScriptRoot
$Target = Join-Path $Repo $Target
$Folder = Join-Path $Repo "dist/$Name"

function Copy-Required([string] $From, [string] $To) {
    if (-not (Test-Path $From -PathType Leaf)) { throw "$From is missing." }
    Copy-Item $From $To -Force
}

if (Test-Path $Folder) { Remove-Item -Recurse -Force $Folder }
New-Item -ItemType Directory -Force (Join-Path $Folder 'licenses') | Out-Null

# The programs, and next to them the DLLs the game loads at run time:
# Tolk's two, and NVDA's current controller client. The game's .pdb holds
# only file and line numbers, so a crash in client.log names where it
# happened.
foreach ($file in 'soundrts.exe', 'soundrts.pdb', 'soundrts-server.exe',
        'nvdaControllerClient64.dll', 'SAAPI64.dll', 'nvdaControllerClient.dll') {
    Copy-Required (Join-Path $Target $file) (Join-Path $Folder $file)
}

Copy-Required (Join-Path $Repo 'LICENSE.txt') (Join-Path $Folder 'LICENSE.txt')
Copy-Required (Join-Path $Repo 'packaging/README.txt') (Join-Path $Folder 'README.txt')

# Tolk is compiled into the game. Its license, and the one of the NVDA
# controller client it brings, come with the tolk-sys crate.
$metadata = cargo metadata --format-version 1 --locked --manifest-path (Join-Path $Repo 'Cargo.toml') | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE" }
$tolk = $metadata.packages | Where-Object name -EQ 'tolk-sys'
if (-not $tolk) { throw 'The tolk-sys crate is not among the dependencies.' }
$tolkSource = Join-Path (Split-Path -Parent $tolk.manifest_path) 'vendor/tolk'
Copy-Required (Join-Path $tolkSource 'LICENSE.txt') (Join-Path $Folder 'licenses/Tolk.txt')
Copy-Required (Join-Path $tolkSource 'LICENSE-NVDA.txt') (Join-Path $Folder 'licenses/nvdaControllerClient64.txt')
Copy-Required (Join-Path $Repo 'vendor/nvda-controller/license.txt') (Join-Path $Folder 'licenses/nvdaControllerClient.txt')

Write-Output "The download's files are in $Folder."
