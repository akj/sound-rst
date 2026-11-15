# The helper of the verify-soundrts skill. SKILL.md, next to this file,
# says when to run each command.
#
#   pwsh -NoProfile -File .claude/skills/verify-soundrts/verify.ps1 <command>
#
#   launch                                   fetch the game data, build, then doctor
#   doctor                                   is this checkout ready to drive?
#   test <feature> <test-file> [<name>...]   drive the in-process harness
#   first-start                              drive the built program from an empty data folder
#   release-check                            build a release folder and check it
#   cleanup                                  remove the scratch files; the evidence stays
#
# The game is started only with --probe or --fetch-data, and with APPDATA
# set to a scratch folder: it does not speak, open a window, read the
# keyboard or touch the player's settings. Every command waits for the
# programs it starts.

param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateSet('launch', 'doctor', 'test', 'first-start', 'release-check', 'cleanup')]
    [string] $Command,
    [Parameter(ValueFromRemainingArguments)]
    [string[]] $Rest = @()
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$Repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$Upstream = Join-Path $Repo 'vendor\soundrts-upstream'
$Pin = (Get-Content (Join-Path $Repo 'scripts/upstream-commit.txt') -Raw).Trim()
$Stage = 'soundrts-verify'

# What each kind of proof exercises. Every evidence file starts with one of
# these, so that a test on recorded devices is never read as a proof of
# what a screen reader says.
$HarnessLayer = 'harness: the client''s own code in a test process, on a recorded speech device, a recorded audio device, scripted keys and a manual clock. This is the text the client sends to the speech device. It is not the window, the audio device, or what a screen reader says.'
$ProgramLayer = 'program: the built soundrts.exe, started with --probe or --fetch-data only. It finds or fetches its data and reports what it found. It shows no menu and no game, because nothing can press its keys.'

# Evidence is kept for each commit, and apart for a checkout with changes.
$Changed = @(git -C $Repo status --porcelain)
$Run = (git -C $Repo rev-parse --short HEAD).Trim() + $(if ($Changed.Count -gt 0) { '-dirty' } else { '' })
$Evidence = Join-Path $Repo "target\verify-soundrts\$Run"
$RepoHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
        [Text.Encoding]::UTF8.GetBytes($Repo.ToLowerInvariant()))).Substring(0, 8)
$Scratch = Join-Path ([IO.Path]::GetTempPath()) "verify-soundrts-$RepoHash"
$Log = $null

# Starts the evidence file of a command with what it proves and of which code.
function New-Log([string] $Path, [string] $Layer, [string] $About) {
    New-Item -ItemType Directory -Force (Split-Path -Parent $Path) | Out-Null
    Set-Content -Path $Path -Value @(
        $About
        "layer: $Layer"
        "commit: $((git -C $Repo rev-parse HEAD).Trim())"
        "changed files: $(if ($Changed.Count -gt 0) { $Changed -join '; ' } else { 'none' })"
        "started: $([DateTime]::UtcNow.ToString('u'))"
    )
    $script:Log = $Path
}

function Write-Result([string] $Line) {
    if ($Log) { Add-Content -Path $Log -Value "`n$Line" }
    Write-Host $Line
}

# Runs a console program in the repository, shows what it prints, adds it to
# the evidence file, and returns its exit code.
function Invoke-Logged([string] $Program, [string[]] $Arguments) {
    $line = "> $Program $($Arguments -join ' ')"
    Write-Host $line
    if ($Log) { Add-Content -Path $Log -Value "`n$line" }
    & $Program @Arguments 2>&1 | ForEach-Object {
        $text = "$_"
        Write-Host $text
        if ($Log) { Add-Content -Path $Log -Value $text }
    }
    $code = $LASTEXITCODE
    if ($Log) { Add-Content -Path $Log -Value "exit code: $code" }
    $code
}

function Assert-Ran([string] $Program, [string[]] $Arguments) {
    $code = Invoke-Logged $Program $Arguments
    if ($code -ne 0) { throw "$Program $($Arguments -join ' ') ended with exit code $code." }
}

# A new, empty scratch folder for one command.
function New-Scratch([string] $Name) {
    $folder = Join-Path $Scratch $Name
    if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
    New-Item -ItemType Directory -Force $folder | Out-Null
    $folder
}

# Where Cargo puts the debug build of the game.
function Get-Game {
    $metadata = cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE." }
    Join-Path $metadata.target_directory 'debug\soundrts.exe'
}

# Runs the game to its end, in $Folder, with $AppData as %APPDATA% and
# without SOUNDRTS_ROOT, and returns its exit code and what it printed. The
# game is a windowed program: it prints only to a stream it is given, and a
# shell does not wait for it unless told to. A game that still runs after
# $Seconds is stopped.
function Invoke-Game([string] $Game, [string[]] $Arguments, [string] $AppData, [string] $Folder, [int] $Seconds) {
    $out = Join-Path $Folder 'out.txt'
    $err = Join-Path $Folder 'err.txt'
    $saved = @{ APPDATA = $env:APPDATA; SOUNDRTS_ROOT = $env:SOUNDRTS_ROOT }
    $env:APPDATA = $AppData
    $env:SOUNDRTS_ROOT = $null
    try {
        $process = Start-Process -FilePath $Game -ArgumentList $Arguments -PassThru -NoNewWindow `
            -WorkingDirectory $Folder -RedirectStandardOutput $out -RedirectStandardError $err
    }
    finally {
        foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
    # Without the handle, the exit code of a process that was not waited for with -Wait is lost.
    $null = $process.Handle
    if (-not $process.WaitForExit($Seconds * 1000)) {
        $process.Kill($true)
        throw "soundrts.exe $($Arguments -join ' ') still ran after $Seconds seconds. It was stopped."
    }
    $process.WaitForExit()
    $text = ((Get-Content $out -Raw) + (Get-Content $err -Raw)).TrimEnd()
    $shown = @("> soundrts.exe $($Arguments -join ' ')", "  (in $Folder, with APPDATA=$AppData)", $text, "exit code: $($process.ExitCode)")
    $shown | ForEach-Object { Write-Host $_ }
    if ($Log) { Add-Content -Path $Log -Value (@('') + $shown) }
    [pscustomobject]@{ ExitCode = $process.ExitCode; Text = $text }
}

# The value of a line of the probe's report, such as "game root".
function Get-ProbeValue([string] $Report, [string] $Name) {
    $line = $Report -split "`r?`n" | Where-Object { $_.StartsWith("${Name}: ") } | Select-Object -First 1
    if ($line) { $line.Substring($Name.Length + 2) } else { '' }
}

function Invoke-Launch {
    Assert-Ran 'pwsh' @('-NoProfile', '-File', 'scripts/fetch-upstream.ps1')
    Assert-Ran 'cargo' @('build', '--locked', '-p', 'soundrts')
    Assert-Ran 'cargo' @('test', '--locked', '-p', 'soundrts', '--no-run')
    Invoke-Doctor
}

function Invoke-Doctor {
    New-Log (Join-Path $Evidence 'doctor.log') "$HarnessLayer`n       $ProgramLayer" 'doctor'

    $want = (Select-String -Path (Join-Path $Repo 'rust-toolchain.toml') -Pattern 'channel\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
    $have = "$(rustc --version)"
    if ($have -notlike "rustc $want *") { throw "rust-toolchain.toml pins Rust $want, and rustc answers '$have'." }
    Write-Result "ok: Rust $want"

    $head = if (Test-Path (Join-Path $Upstream '.git')) { "$(git -C $Upstream rev-parse HEAD)".Trim() } else { 'nothing' }
    if ($head -ne $Pin -or -not (Test-Path (Join-Path $Upstream 'res/rules.txt'))) {
        throw "The game data in $Upstream is at $head, and scripts/upstream-commit.txt pins $Pin. Run the launch command."
    }
    Write-Result "ok: the game data is the pinned commit $Pin"

    # The built program starts and finds that data.
    Assert-Ran 'cargo' @('build', '--locked', '-p', 'soundrts')
    $folder = New-Scratch 'doctor'
    try {
        $probe = Invoke-Game (Get-Game) @('--root', "`"$Upstream`"", '--probe') (Join-Path $folder 'appdata') $folder 60
    }
    finally { Remove-Item -Recurse -Force $folder }
    $root = Get-ProbeValue $probe.Text 'game root'
    if ($probe.ExitCode -ne 0 -or $root -ne $Upstream) {
        throw "soundrts.exe --probe ended with exit code $($probe.ExitCode) and found its data in '$root', not in '$Upstream'."
    }
    Write-Result "ok: the built program starts and finds the game data. It would speak through: $(Get-ProbeValue $probe.Text 'speaking through')"

    # The harness builds a whole client on that data and walks the main menu.
    $smoke = 'the_main_menu_says_its_title_and_the_arrows_walk_its_items'
    Assert-Ran 'cargo' @('test', '--locked', '-p', 'soundrts', '--test', 'menus', '--', '--exact', $smoke)
    if ((Get-Content $Log -Raw) -notmatch 'test result: ok\. 1 passed') { throw "The harness test $smoke did not run." }
    Write-Result 'ok: the harness runs a client on recorded devices'
    Write-Result "doctor: ready. Evidence: $Log"
}

function Invoke-Test {
    if ($Rest.Count -lt 2) { throw 'usage: verify.ps1 test <feature> <test-file> [<test-name>...]' }
    $feature, $testFile = $Rest[0], $Rest[1]
    $names = @($Rest | Select-Object -Skip 2)
    if (-not (Test-Path (Join-Path $PSScriptRoot "features/$feature.md"))) {
        throw "No feature '$feature': the features are the files of $(Join-Path $PSScriptRoot 'features')."
    }
    if (-not (Test-Path (Join-Path $Repo "crates/client/tests/$testFile.rs"))) {
        throw "No test file crates/client/tests/$testFile.rs."
    }
    New-Log (Join-Path $Evidence "$feature\$testFile.log") $HarnessLayer "feature: $feature"

    $arguments = @('test', '--locked', '-p', 'soundrts', '--test', $testFile)
    if ($names.Count -gt 0) { $arguments += @('--', '--exact') + $names }
    $started = Get-Date
    Assert-Ran 'cargo' $arguments

    # cargo test succeeds when a name matches no test, so count what ran.
    $passed = if ((Get-Content $Log -Raw) -match 'test result: ok\. (\d+) passed') { [int] $Matches[1] } else { 0 }
    if ($passed -eq 0) { throw "No test of $testFile ran." }
    if ($names.Count -gt 0 -and $passed -ne $names.Count) {
        throw "$passed of the $($names.Count) named tests ran: a name matches no test of $testFile."
    }

    # The transcripts say what the scripted player pressed and what the client said.
    if ($testFile -eq 'transcripts') {
        $kept = Join-Path $Evidence "$feature\transcripts"
        New-Item -ItemType Directory -Force $kept | Out-Null
        Get-ChildItem (Join-Path $Repo 'target/transcripts') -Filter '*.txt' |
            Where-Object { $_.LastWriteTime -ge $started -and $_.Name -notlike '*.sounds.txt' } |
            ForEach-Object {
                Copy-Item $_.FullName $kept -Force
                Write-Result "transcript: $(Join-Path $kept $_.Name)"
            }
    }
    Write-Result "passed on the harness: $testFile for $feature (tests run: $passed). Evidence: $Log"
}

function Invoke-FirstStart {
    New-Log (Join-Path $Evidence 'first-start\first-start.log') $ProgramLayer 'feature: first-start'
    Assert-Ran 'cargo' @('build', '--locked', '-p', 'soundrts')
    $game = Get-Game
    $folder = New-Scratch 'first-start'
    try {
        $appData = Join-Path $folder 'appdata'
        New-Item -ItemType Directory $appData | Out-Null
        $root = Join-Path $appData "SoundRTS\data\$Pin"

        $before = Invoke-Game $game @('--probe') $appData $folder 60
        if ($before.ExitCode -eq 0 -or -not $before.Text.Contains('soundrts --fetch-data')) {
            throw 'Without game data, --probe must fail and say to run soundrts --fetch-data.'
        }
        Write-Result 'ok: without game data, the program stops and says how to get it'

        $fetch = Invoke-Game $game @('--fetch-data') $appData $folder 600
        if ($fetch.ExitCode -ne 0 -or -not $fetch.Text.Contains("The game data is ready in $root")) {
            throw "--fetch-data did not report the game data ready in $root."
        }
        $same = (Get-FileHash (Join-Path $root 'res/rules.txt')).Hash -eq (Get-FileHash (Join-Path $Upstream 'res/rules.txt')).Hash
        if (-not $same) { throw "res/rules.txt in $root differs from the one in $Upstream." }
        Write-Result 'ok: --fetch-data downloaded the game data, with the res/rules.txt of the checkout'

        $after = Invoke-Game $game @('--probe') $appData $folder 60
        $found = Get-ProbeValue $after.Text 'game root'
        $user = Get-ProbeValue $after.Text 'user folder'
        if ($after.ExitCode -ne 0 -or $found -ne $root -or -not $user.StartsWith($appData)) {
            throw "--probe found its data in '$found' and its settings in '$user', not under $appData."
        }
        Write-Result 'ok: the program finds the fetched data, and keeps its settings in the scratch folder'

        $again = Invoke-Game $game @('--fetch-data') $appData $folder 60
        if ($again.ExitCode -ne 0 -or -not $again.Text.Contains("The game data is already in $root")) {
            throw 'A second --fetch-data must find the data and download nothing.'
        }
        Write-Result 'ok: a second --fetch-data downloads nothing'
    }
    finally { Remove-Item -Recurse -Force $folder }
    Write-Result "passed on the built program: first-start. Evidence: $Log"
}

function Invoke-ReleaseCheck {
    New-Log (Join-Path $Evidence 'first-start\release-check.log') $ProgramLayer 'feature: first-start, as a release folder'
    $work = New-Scratch 'release-check'
    try {
        Assert-Ran 'pwsh' @('-NoProfile', '-File', 'scripts/fetch-nvda-controller.ps1')
        Assert-Ran 'cargo' @('build', '--release', '--locked', '-p', 'soundrts', '-p', 'soundrts-server')
        Assert-Ran 'pwsh' @('-NoProfile', '-File', 'scripts/stage-release.ps1', '-Name', $Stage)
        Assert-Ran 'pwsh' @('-NoProfile', '-File', 'scripts/check-release.ps1', '-Folder', "dist/$Stage", '-Work', (Join-Path $work 'check'))
    }
    finally {
        Remove-Item -Recurse -Force $work
        $staged = Join-Path $Repo "dist\$Stage"
        if (Test-Path $staged) { Remove-Item -Recurse -Force $staged }
    }
    Write-Result "passed on the built program: the release folder. Evidence: $Log"
}

function Invoke-Cleanup {
    $staged = Join-Path $Repo "dist\$Stage"
    foreach ($folder in $Scratch, $staged) {
        if (Test-Path $folder) {
            Remove-Item -Recurse -Force $folder
            Write-Host "removed $folder"
        }
    }
    if (-not (Test-Path $Evidence)) {
        Write-Host "No evidence for $Run in $Evidence."
        return
    }
    Write-Host "The evidence for $Run stays in ${Evidence}:"
    Get-ChildItem $Evidence -Recurse -File | ForEach-Object { Write-Host "  $([IO.Path]::GetRelativePath($Evidence, $_.FullName))" }
}

# Cargo and the tests print UTF-8.
$encoding = [Console]::OutputEncoding
[Console]::OutputEncoding = [Text.Encoding]::UTF8
Push-Location $Repo
try {
    switch ($Command) {
        'launch' { Invoke-Launch }
        'doctor' { Invoke-Doctor }
        'test' { Invoke-Test }
        'first-start' { Invoke-FirstStart }
        'release-check' { Invoke-ReleaseCheck }
        'cleanup' { Invoke-Cleanup }
    }
}
catch {
    Write-Result "FAILED: $($_.Exception.Message)"
    exit 1
}
finally {
    Pop-Location
    [Console]::OutputEncoding = $encoding
}
