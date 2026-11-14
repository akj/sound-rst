# Plays the sound candidates role by role, in random order without saying
# which is which, and asks for a rating from 1 to 5 after each. At the end of
# each role it says which version was which candidate. The answers are saved
# after every answer in vendor/sound-candidates/ratings/. Everything goes
# through plain lines of text, so a screen reader reads the questions and you
# type the answers. Run scripts/fetch-sound-candidates.ps1 first.
#
#   pwsh scripts/rate-sounds.ps1                        every role, footsteps first
#   pwsh scripts/rate-sounds.ps1 footsteps-*            only the footsteps
#   pwsh scripts/rate-sounds.ps1 ambience-town          one role
#
# -Seconds sets how much of each walk or bed plays (20 by default). -Seed
# repeats the random order of an earlier session. -NoPlay prepares the audio
# and asks the questions without playing anything.

param(
    [string[]]$Role = @('*'),
    [double]$Seconds = 20,
    [int]$Seed = (Get-Random -Maximum 1000000),
    [switch]$NoPlay
)

$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent $PSScriptRoot
$Manifest = Import-PowerShellDataFile (Join-Path $PSScriptRoot 'sound-candidates.psd1')
$Rendered = Join-Path $Root 'vendor/sound-candidates/.audition'

# What each role is for, as the question names it, and what to listen for.
$Purpose = @{
    'footsteps-grass'       = @('footsteps on grass', '')
    'footsteps-leaves'      = @('footsteps on a forest floor', '')
    'footsteps-gravel'      = @('footsteps on stone, a road or gravel', 'None of these is as bright as the others, so judge whether any is usable.')
    'footsteps-wood'        = @('footsteps on wood, such as a bridge', 'Both are dull; judge whether either is usable.')
    'footsteps-mud'         = @('footsteps in mud', '')
    'footsteps-water'       = @('footsteps in shallow water, such as a ford', '')
    'footsteps-snow'        = @('footsteps in snow', '')
    'ambience-meadow'       = @('the background of a meadow', 'Listen for whether it could be told from the forest.')
    'ambience-forest'       = @('the background of a forest', 'Listen for whether it could be told from the meadow.')
    'ambience-dense-forest' = @('the background of a dense forest', 'Listen for whether it could be told from the mountain, since both have wind.')
    'ambience-mountain'     = @('the background of a mountain or mountain pass', 'Listen for whether it could be told from the dense forest.')
    'ambience-marsh'        = @('the background of a marsh', 'Listen for whether it could be told from a forest at night.')
    'ambience-river'        = @('the background of a river', '')
    'ambience-lake'         = @('the background of a lake', '')
    'ambience-sea'          = @('the background of the sea or the ocean', 'Note in the comment which would suit the sea and which the ocean.')
    'ambience-town'         = @('the background of a town', 'Clear speech would rule a candidate out.')
}

# pwsh -File passes "a,b" as one string.
$Role = @($Role -split ',' | ForEach-Object Trim | Where-Object { $_ })
$roles = @($Manifest.Candidates | ForEach-Object { $_.Role } | Select-Object -Unique |
    Where-Object { $r = $_; @($Role | Where-Object { $r -like $_ }).Count -gt 0 })
if ($roles.Count -eq 0) { throw "No role matches $($Role -join ', '). Run pwsh scripts/audition-sounds.ps1 to list the roles." }

$folder = Join-Path $Root 'vendor/sound-candidates/ratings'
New-Item -ItemType Directory -Force $folder | Out-Null
$ResultsPath = Join-Path $folder ("ratings-" + (Get-Date -Format 'yyyy-MM-dd-HHmmss') + '.txt')
$Header = @(
    'Sound ratings'
    "Started $(Get-Date -Format 'yyyy-MM-dd HH:mm'). Roles: $($roles -join ', ')."
    "Seed: $Seed. Run again with -Seed $Seed to hear the same order."
)
$Steps = [Collections.Generic.List[object]]::new()
$Random = [Random]::new($Seed)

function Say([string]$Text) { Write-Host $Text }

# Asks a question and returns the answer, trimmed. q, or the end of the
# input, stops the session.
function Ask([string]$Prompt) {
    Say $Prompt
    $line = [Console]::In.ReadLine()
    if ($null -eq $line -or $line.Trim() -in 'q', 'quit') { throw [OperationCanceledException]::new() }
    return $line.Trim()
}

function Get-Summary {
    foreach ($step in $Steps) {
        if ($step.Ratings.Count -eq 0) { continue }
        $top = ($step.Ratings | Measure-Object Rating -Maximum).Maximum
        $best = @($step.Ratings | Where-Object Rating -eq $top | ForEach-Object { "candidate $($_.Number), $($_.Title), by $($_.Author)" })
        "$($step.Role): best rated $top out of 5, " + ($best -join '; and ')
    }
}

function Save-Results {
    $lines = @($Header)
    foreach ($step in $Steps) { $lines += ''; $lines += $step.Lines }
    $lines += ''
    $lines += 'Summary'
    $lines += @(Get-Summary)
    Set-Content -Path $ResultsPath -Value $lines -Encoding utf8
}

function Play([string]$Path) {
    if ($NoPlay) { Say '(Not playing: -NoPlay.)'; return }
    # Lets the screen reader finish the last line before the sound starts.
    Start-Sleep -Seconds 1
    & ffplay -nodisp -autoexit -loglevel quiet $Path
}

function Invoke-Role([int]$Number, [string]$Name) {
    $list = @($Manifest.Candidates | Where-Object Role -eq $Name)
    $what, $hint = $Purpose[$Name]
    if (-not $what) { $what = $Name; $hint = '' }
    $step = [pscustomobject]@{
        Role    = $Name
        Lines   = [Collections.Generic.List[string]]::new()
        Ratings = [Collections.Generic.List[object]]::new()
    }
    $step.Lines.Add("Step ${Number}: $Name")
    # The step is in the results from the start, so stopping part way keeps
    # the answers already given.
    $Steps.Add($step)
    Save-Results

    Say ''
    Say "Step $Number of $($roles.Count): $what. $($list.Count) versions, in random order. $hint".TrimEnd()
    Say 'Preparing the sounds.'
    & (Join-Path $PSScriptRoot 'audition-sounds.ps1') -Role $Name -Seconds $Seconds -NoPlay | Out-Null

    $order = 1..$list.Count | ForEach-Object { $_ }
    for ($i = $order.Count - 1; $i -gt 0; $i--) {
        $j = $Random.Next($i + 1)
        $order[$i], $order[$j] = $order[$j], $order[$i]
    }
    $given = @{}
    for ($v = 1; $v -le $order.Count; $v++) {
        $n = $order[$v - 1]
        $entry = $list[$n - 1]
        $wav = Join-Path $Rendered "$Name-$n.wav"
        $kind = if ($entry.Kind -eq 'steps') { 'single steps played as a walk' } elseif ($entry.Kind -eq 'walk') { 'a walking recording' } else { 'a recording' }
        Ask "Version $v of $($order.Count), $kind. Press Enter to play it." | Out-Null
        Play $wav
        while ($true) {
            $answer = Ask "How good would this be as ${what}? Type a number from 1, unusable, to 5, just right, or r to hear it again."
            if ($answer -eq 'r') { Play $wav; continue }
            if ($answer -match '^[1-5]$') { $rating = [int]$answer; break }
            Say 'Please type a number from 1 to 5, or r.'
        }
        $given[$v] = $rating
        $step.Ratings.Add([pscustomobject]@{ Number = $n; Rating = $rating; Title = $entry.Title; Author = $entry.Author })
        $step.Lines.Add("  Version $v, candidate $n ($($entry.Title), by $($entry.Author)): $rating out of 5")
        Save-Results
    }
    for ($v = 1; $v -le $order.Count; $v++) {
        $entry = $list[$order[$v - 1] - 1]
        $shares = @($entry.Files | ForEach-Object { $_.Above4k } | Sort-Object)
        $bright = if ($shares.Count -gt 1) { "$($shares[0]) to $($shares[-1])" } else { "$($shares[0])" }
        Say "Version $v, which you rated $($given[$v]), was candidate $($order[$v - 1]): $($entry.Title), by $($entry.Author); $bright percent of its energy above 4 kilohertz."
    }
    $comment = Ask 'Any comment on this step? Type it and press Enter, or press Enter to go on.'
    if ($comment) { $step.Lines.Add("  Comment: $comment"); Save-Results }
}

Say 'Sound ratings. It plays each role''s candidates in random order and asks you to rate each from 1 to 5. Use headphones.'
Say "Your answers are saved after each answer in $ResultsPath. Type q at any question to stop."
Save-Results
try {
    for ($k = 0; $k -lt $roles.Count; $k++) { Invoke-Role ($k + 1) $roles[$k] }
}
catch [OperationCanceledException] {
    Save-Results
    Say "Stopped. Your answers so far are in $ResultsPath."
    exit 0
}
Save-Results
Say ''
Say 'That is the end of the script.'
foreach ($line in @(Get-Summary)) { Say $line }
Say "Your answers are in $ResultsPath."
