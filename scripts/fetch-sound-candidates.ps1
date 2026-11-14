# Fetches the CC0 sound candidates listed in scripts/sound-candidates.psd1
# into vendor/sound-candidates, one folder per role and candidate, and checks
# every file against its recorded SHA-256. Beside each candidate's files it
# writes source.txt with the page, author and licence. Safe to rerun: files
# that are already there with the right hash are kept.
#
# Freesound gives original files only to a signed-in user, through OAuth2.
# Pass a Freesound access token with -FreesoundToken (or $env:FREESOUND_TOKEN).
# Without one, and when a Freesound file is missing, the script prints the
# address where you approve access and asks for the code Freesound shows; for
# that it needs your API client id (-FreesoundClientId or
# $env:FREESOUND_CLIENT_ID) from https://freesound.org/apiv2/apply.
#
# Every file listed is CC0, as recorded in docs/sound-sources.md. The files
# stay out of the repository.

param(
    [string]$FreesoundToken = $env:FREESOUND_TOKEN,
    [string]$FreesoundClientId = $env:FREESOUND_CLIENT_ID
)

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$Manifest = Import-PowerShellDataFile (Join-Path $PSScriptRoot 'sound-candidates.psd1')
$Target = Join-Path $Root 'vendor/sound-candidates'
$Agent = 'sound-candidates-fetch/1.0'
$RedirectUri = 'https://freesound.org/home/app_permissions/permission_granted/'

function Get-Hash([string]$Path) { (Get-FileHash $Path -Algorithm SHA256).Hash }

function Get-FreesoundToken {
    if ($script:FreesoundToken) { return $script:FreesoundToken }
    if (-not $FreesoundClientId) {
        throw 'A Freesound file is missing. Pass -FreesoundToken, or -FreesoundClientId to approve access now.'
    }
    # OAuth2 with PKCE: Freesound shows the code on its own page, to paste here.
    $bytes = [byte[]]::new(48)
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    $verifier = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    $digest = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::ASCII.GetBytes($verifier))
    $challenge = [Convert]::ToBase64String($digest).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    $url = "https://freesound.org/apiv2/oauth2/authorize/?client_id=$FreesoundClientId&response_type=code" +
        "&code_challenge=$challenge&code_challenge_method=S256&redirect_uri=$RedirectUri"
    # Write-Host, because this function's output is the token.
    Write-Host 'Open this address, sign in to Freesound if asked, and choose Authorize:'
    Write-Host $url
    $code = Read-Host 'Then paste the code Freesound shows'
    $reply = Invoke-RestMethod -Method Post -UserAgent $Agent -Uri 'https://freesound.org/apiv2/oauth2/access_token/' -Body @{
        client_id     = $FreesoundClientId
        grant_type    = 'authorization_code'
        code          = $code.Trim()
        code_verifier = $verifier
        redirect_uri  = $RedirectUri
    }
    $script:FreesoundToken = $reply.access_token
    return $script:FreesoundToken
}

function Save-Download([hashtable]$File, [string]$Path) {
    if ($File.FreesoundId) {
        $token = Get-FreesoundToken
        Invoke-WebRequest -UserAgent $Agent -Headers @{ Authorization = "Bearer $token" } `
            -Uri "https://freesound.org/apiv2/sounds/$($File.FreesoundId)/download/" -OutFile $Path
        # Freesound limits each account to 60 requests a minute.
        Start-Sleep -Seconds 2
    }
    elseif ($File.ZipEntry) {
        # One download per archive per run, kept until the run ends.
        $zip = $script:Zips[$File.Url]
        if (-not $zip) {
            $zip = Join-Path $Work ("archive-" + $script:Zips.Count + '.zip')
            Invoke-WebRequest -UserAgent $Agent -Uri $File.Url -OutFile $zip
            $script:Zips[$File.Url] = $zip
        }
        $archive = [IO.Compression.ZipFile]::OpenRead($zip)
        try {
            $entry = $archive.GetEntry($File.ZipEntry)
            if (-not $entry) { throw "$($File.Url) has no entry $($File.ZipEntry)." }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $Path, $true)
        }
        finally { $archive.Dispose() }
    }
    else {
        Invoke-WebRequest -UserAgent $Agent -Uri $File.Url -OutFile $Path
    }
}

function Get-Origin([hashtable]$File) {
    if ($File.FreesoundId) { return "https://freesound.org/apiv2/sounds/$($File.FreesoundId)/download/ (Freesound API, sign-in needed)" }
    if ($File.ZipEntry) { return "$($File.ZipEntry) inside $($File.Url)" }
    return $File.Url
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
New-Item -ItemType Directory -Force $Target | Out-Null
$Work = Join-Path $Target '.download'
if (Test-Path $Work) { Remove-Item -Recurse -Force $Work }
New-Item -ItemType Directory $Work | Out-Null
$Zips = @{}
$fetched = 0
$kept = 0

try {
    foreach ($candidate in $Manifest.Candidates) {
        $folder = Join-Path $Target (Join-Path $candidate.Role $candidate.Folder)
        New-Item -ItemType Directory -Force $folder | Out-Null
        $lines = @(
            "Role: $($candidate.Role)"
            "Candidate: $($candidate.Title)"
            "Page: $($candidate.Page)"
            "Author: $($candidate.Author)"
            "Licence, as the source states it: $($candidate.Licence)"
            "Licence checked: $($candidate.LicenceChecked)"
            "Notes: $($candidate.Notes)"
            ''
        )
        foreach ($file in $candidate.Files) {
            $path = Join-Path $folder $file.Name
            if ((Test-Path $path) -and (Get-Hash $path) -eq $file.Sha256) {
                $kept++
            }
            else {
                $partial = Join-Path $Work $file.Name
                Save-Download $file $partial
                $hash = Get-Hash $partial
                if ($hash -ne $file.Sha256) { throw "$(Get-Origin $file) has hash $hash, expected $($file.Sha256)." }
                Move-Item -Force $partial $path
                $fetched++
            }
            $lines += "File: $($file.Name)"
            $lines += "  From: $(Get-Origin $file)"
            $lines += "  SHA-256: $($file.Sha256)"
            $lines += "  $($file.Format), $($file.SampleRate) Hz, $($file.Channels) channel(s), $($file.Seconds) s, $($file.Above4k) percent of energy above 4 kHz"
        }
        Set-Content -Path (Join-Path $folder 'source.txt') -Value $lines -Encoding utf8
    }
}
finally {
    Remove-Item -Recurse -Force $Work
}

Write-Output "Sound candidates are ready at $Target ($fetched fetched, $kept already there)."
