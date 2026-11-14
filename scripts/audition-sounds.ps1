# Plays the sound candidates for one role, one after another, and prints
# before each which candidate it is. Run scripts/fetch-sound-candidates.ps1
# first. Needs ffmpeg and ffplay on the path; ffplay plays with no window.
#
#   pwsh scripts/audition-sounds.ps1                      lists the roles
#   pwsh scripts/audition-sounds.ps1 footsteps-grass      plays every candidate
#   pwsh scripts/audition-sounds.ps1 footsteps-grass 2    plays candidate 2 only
#
# Every candidate is brought to the same loudness first, so a louder file does
# not win by being louder. Footsteps are mixed to mono, as the game would play
# them; a set of single steps is played as a walk, each step half a second
# apart, twice through. Ambience plays in stereo for -Seconds (20 by default).
# -NoPlay prepares the audio and prints the lines without playing anything.

param(
    [string]$Role,
    [int]$Candidate,
    [double]$Seconds = 20,
    [switch]$NoPlay
)

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$Manifest = Import-PowerShellDataFile (Join-Path $PSScriptRoot 'sound-candidates.psd1')
$Fetched = Join-Path $Root 'vendor/sound-candidates'
$Rendered = Join-Path $Fetched '.audition'

$roles = $Manifest.Candidates | ForEach-Object { $_.Role } | Select-Object -Unique
if (-not $Role) {
    Write-Output 'Roles with candidates:'
    foreach ($r in $roles) {
        $count = @($Manifest.Candidates | Where-Object Role -eq $r).Count
        Write-Output "  $r, $count candidate(s)"
    }
    Write-Output 'To hear one role: pwsh scripts/audition-sounds.ps1 ROLE'
    exit 0
}
if ($roles -notcontains $Role) { throw "No role named $Role. Run the script with no arguments to list the roles." }

$list = @($Manifest.Candidates | Where-Object Role -eq $Role)
if ($Candidate -and ($Candidate -lt 1 -or $Candidate -gt $list.Count)) { throw "$Role has candidates 1 to $($list.Count)." }

function Invoke-Ffmpeg {
    $output = & ffmpeg -hide_banner -nostdin -y @args 2>&1
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed: $($output | Select-Object -Last 3)" }
    return $output
}

# Renders a candidate to one WAV file at a common loudness, and says what it is.
function New-Audition([hashtable]$Entry, [string]$Out) {
    $folder = Join-Path $Fetched (Join-Path $Entry.Role $Entry.Folder)
    $paths = @(foreach ($file in $Entry.Files) {
        $path = Join-Path $folder $file.Name
        if (-not (Test-Path $path)) { throw "$path is missing. Run pwsh scripts/fetch-sound-candidates.ps1 first." }
        $path
    })
    $raw = "$Out.raw.wav"
    if ($Entry.Kind -eq 'steps') {
        $inputs = foreach ($p in ($paths + $paths)) { '-i'; $p }
        $chains = for ($i = 0; $i -lt 2 * $paths.Count; $i++) {
            "[$i]aresample=48000,aformat=channel_layouts=mono,apad=whole_dur=0.5,atrim=0:0.5[s$i]"
        }
        $labels = (0..(2 * $paths.Count - 1) | ForEach-Object { "[s$_]" }) -join ''
        $graph = ($chains -join ';') + ";${labels}concat=n=$(2 * $paths.Count):v=0:a=1[out]"
        Invoke-Ffmpeg @inputs -filter_complex $graph -map '[out]' $raw | Out-Null
        $what = "a walk of $($paths.Count) single steps, twice through"
    }
    else {
        $start = if ($Entry.Start) { $Entry.Start } else { 0 }
        $layout = if ($Entry.Kind -eq 'walk') { 'mono' } else { 'stereo' }
        Invoke-Ffmpeg -ss $start -t $Seconds -i $paths[0] -af "aresample=48000,aformat=channel_layouts=$layout" $raw | Out-Null
        $what = if ($Entry.Kind -eq 'walk') { "up to $Seconds seconds of a walking recording" } else { "up to $Seconds seconds of the recording" }
    }
    # Measure loudness, then apply one fixed gain to reach -23 LUFS.
    $log = Invoke-Ffmpeg -i $raw -af ebur128=framelog=quiet -f null -
    $loudness = [double](($log | Select-String '^\s*I:\s*(-?[\d.]+) LUFS' | Select-Object -Last 1).Matches.Groups[1].Value)
    $gain = [Math]::Min(30, -23 - $loudness)
    Invoke-Ffmpeg -i $raw -af "volume=${gain}dB,alimiter=limit=0.9:level=disabled" $Out | Out-Null
    Remove-Item $raw
    return $what
}

New-Item -ItemType Directory -Force $Rendered | Out-Null
$numbers = if ($Candidate) { @($Candidate) } else { 1..$list.Count }
foreach ($n in $numbers) {
    $entry = $list[$n - 1]
    $out = Join-Path $Rendered "$Role-$n.wav"
    $what = New-Audition $entry $out
    $first = $entry.Files[0]
    $channels = switch ($first.Channels) { 1 { 'mono' } 2 { 'stereo' } default { "$_ channels" } }
    Write-Output ''
    Write-Output "Candidate $n of $($list.Count) for $Role`: $($entry.Title), by $($entry.Author)."
    $shares = @($entry.Files | ForEach-Object { $_.Above4k } | Sort-Object)
    $bright = if ($shares.Count -gt 1) { "$($shares[0]) to $($shares[-1]) percent of each step's energy is" } else { "$($shares[0]) percent of its energy is" }
    Write-Output "Recorded $channels at $($first.SampleRate) hertz; $bright above 4 kilohertz."
    if ($entry.Notes) { Write-Output $entry.Notes }
    Write-Output "Playing $what."
    if (-not $NoPlay) {
        & ffplay -nodisp -autoexit -loglevel quiet $out
        Start-Sleep -Milliseconds 800
    }
}
Write-Output ''
Write-Output "Done with $Role. To hear one again: pwsh scripts/audition-sounds.ps1 $Role NUMBER"
