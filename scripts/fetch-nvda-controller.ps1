# Fetches NVDA's controller client, a pinned release checked by its hash,
# into vendor/nvda-controller. With the nvda_direct setting, the game speaks
# through it and learns from NVDA when speech ends. The build copies the DLL
# next to the program when it is there. Safe to rerun.
#
# The controller client is LGPL 2.1, from NV Access. Its license is fetched
# with it.

$ErrorActionPreference = 'Stop'

$Version = '2026.3.1'
$Url = "https://download.nvaccess.org/releases/$Version/nvda_${Version}_controllerClient.zip"
$ZipHash = 'AEDEDBF58C7EBFE5ACB8CE1B52B4F0CC017F5E604F52DE32EEF69AC8EB0310B2'
$DllHash = '8C8D9E1952C8B52820988081EA58C26175E69C0FBB5A09E8F2034AAE70D55F0A'
$Target = Join-Path (Split-Path -Parent $PSScriptRoot) 'vendor/nvda-controller'
$Dll = Join-Path $Target 'nvdaControllerClient.dll'

if ((Test-Path $Dll) -and (Get-FileHash $Dll -Algorithm SHA256).Hash -eq $DllHash) {
    Write-Output "NVDA controller client $Version is ready at $Dll."
    exit 0
}

New-Item -ItemType Directory -Force $Target | Out-Null
$Work = Join-Path $Target 'download'
if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
New-Item -ItemType Directory $Work | Out-Null
try {
    $Zip = Join-Path $Work 'controllerClient.zip'
    Invoke-WebRequest -Uri $Url -OutFile $Zip
    $hash = (Get-FileHash $Zip -Algorithm SHA256).Hash
    if ($hash -ne $ZipHash) { throw "$Url has hash $hash, expected $ZipHash." }

    $Extracted = Join-Path $Work 'zip'
    Expand-Archive -Path $Zip -DestinationPath $Extracted
    $hash = (Get-FileHash (Join-Path $Extracted 'x64/nvdaControllerClient.dll') -Algorithm SHA256).Hash
    if ($hash -ne $DllHash) { throw "x64/nvdaControllerClient.dll has hash $hash, expected $DllHash." }

    Copy-Item (Join-Path $Extracted 'license.txt') (Join-Path $Target 'license.txt') -Force
    Copy-Item (Join-Path $Extracted 'x64/nvdaControllerClient.dll') $Dll -Force
}
finally {
    Remove-Item -Recurse -Force $Work
}

Write-Output "NVDA controller client $Version is ready at $Dll."
