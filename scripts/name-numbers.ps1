# Keeps crates/import/src/original_names.txt, the table that gives a name
# to each number of the original's files: a line of ui/tts.txt (a phrase)
# or a recording under ui/ (a sound).
#
# The script reads the original's checkout and the code. It adds a row with
# a proposed name for each number that has none, and writes again the
# `unused` mark and the comment of every row. It never changes a name that
# the table already has, so a name that a person corrected stays. To get a
# new proposal for a number, delete its row. Safe to rerun: a second run
# changes nothing.
#
# With -CopySoundsTo <folder>, it also copies each recording to that folder
# under its name, as <folder>/<package>/<name>.ogg, so that a listener can
# play each file and compare it with its name.

param(
    [string]$Upstream,
    [string]$Table,
    [string]$CopySoundsTo
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repo = Split-Path -Parent $PSScriptRoot
if (-not $Upstream) { $Upstream = Join-Path $Repo 'vendor/soundrts-upstream' }
if (-not $Table) { $Table = Join-Path $Repo 'crates/import/src/original_names.txt' }
if (-not (Test-Path (Join-Path $Upstream 'res/ui/tts.txt'))) {
    throw "No original at $Upstream. Run scripts/fetch-upstream.ps1 first."
}

Add-Type -AssemblyName System.IO.Compression.FileSystem

# The text files and the recordings of a package, as relative path -> a
# function that returns the bytes. A package is a folder or a zip.
function Get-PackageFiles([string]$Path) {
    $files = [ordered]@{}
    if (Test-Path $Path -PathType Container) {
        $root = (Resolve-Path $Path).Path
        Get-ChildItem $root -Recurse -File | Sort-Object FullName | ForEach-Object {
            $rel = $_.FullName.Substring($root.Length + 1).Replace('\', '/')
            $files[$rel] = $_.FullName
        }
    } else {
        $zip = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $Path).Path)
        try {
            foreach ($e in ($zip.Entries | Sort-Object FullName)) {
                if ($e.FullName.EndsWith('/')) { continue }
                $stream = $e.Open()
                $memory = [System.IO.MemoryStream]::new()
                $stream.CopyTo($memory)
                $stream.Dispose()
                $files[$e.FullName] = , $memory.ToArray()
            }
        } finally { $zip.Dispose() }
    }
    $files
}

function Get-Bytes($Source) {
    if ($Source -is [string]) { [System.IO.File]::ReadAllBytes($Source) } else { $Source }
}

function Get-Text($Source) {
    $text = [System.Text.Encoding]::UTF8.GetString((Get-Bytes $Source))
    $text.TrimStart([char]0xFEFF)
}

function Test-Number([string]$Word) { $Word -match '^[0-9]+$' }

# The packages: the game, each map that is a folder or a zip, each mod, and
# each campaign. A map that is one text file has no numbers of its own.
$packagePaths = [ordered]@{ 'res' = Join-Path $Upstream 'res' }
foreach ($group in 'res/multi', 'mods', 'single') {
    $dir = Join-Path $Upstream $group
    if (-not (Test-Path $dir)) { continue }
    Get-ChildItem $dir | Sort-Object Name | ForEach-Object {
        if ($_.PSIsContainer) {
            $packagePaths["$group/$($_.Name)"] = $_.FullName
        } elseif ($_.Extension -eq '.zip') {
            $packagePaths["$group/$($_.BaseName)"] = $_.FullName
        }
    }
}

# What refers to a number, by number: "style peasant.attack_hit", "map jl1",
# "help", "code". A use counts for every package, because a mod's style may
# use a number of the game.
$uses = @{}
function Add-Use([string]$Number, [string]$Use) {
    if (-not $uses.ContainsKey($Number)) { $uses[$Number] = [System.Collections.Generic.List[string]]::new() }
    if (-not $uses[$Number].Contains($Use)) { $uses[$Number].Add($Use) }
}

function Read-Style([string]$Text) {
    $def = ''
    foreach ($line in $Text -split "`n") {
        $words = @(($line -split ';')[0].Trim() -split '\s+' | Where-Object { $_ })
        if ($words.Count -eq 0) { continue }
        if ($words[0] -eq 'def') { $def = if ($words.Count -gt 1) { $words[1] } else { '' }; continue }
        foreach ($w in $words | Select-Object -Skip 1) {
            if (Test-Number $w) { Add-Use $w "style $def.$($words[0])" }
        }
    }
}

# The numbers of a map or of a campaign chapter: its title, objective,
# intro and sequence lines, and the cut scenes and objectives of triggers.
function Read-Map([string]$Text, [string]$Name) {
    foreach ($line in $Text -split "`n") {
        $line = ($line -split ';')[0].Trim()
        if ($line -match '^(title|objective|intro|sequence)\s+(.*)$') {
            foreach ($w in $Matches[2] -split '\s+') { if (Test-Number $w) { Add-Use $w "map $Name" } }
        }
        foreach ($m in [regex]::Matches($line, '\(cut_scene\s+([^)]*)\)')) {
            foreach ($w in $m.Groups[1].Value -split '\s+') { if (Test-Number $w) { Add-Use $w "map $Name" } }
        }
        foreach ($m in [regex]::Matches($line, '\(add_objective\s+\S+\s+([^)]*)\)')) {
            foreach ($w in $m.Groups[1].Value -split '\s+') { if (Test-Number $w) { Add-Use $w "map $Name" } }
        }
    }
}

# The numbers each package defines: number -> @{ Kind; English; File }.
$defined = [ordered]@{}
$soundFiles = @{}
foreach ($package in $packagePaths.Keys) {
    $files = Get-PackageFiles $packagePaths[$package]
    $numbers = @{}
    foreach ($rel in $files.Keys) {
        if ($rel -match '^ui(-[^/]+)?/tts\.txt$') {
            $base = -not $Matches[1]
            foreach ($line in (Get-Text $files[$rel]) -split "`n") {
                $line = $line.Trim()
                if ($line -notmatch '^(\S+)\s+(.*)$') { continue }
                $key = $Matches[1]; $english = $Matches[2].Trim()
                if (-not (Test-Number $key)) { continue }
                if (-not $numbers.ContainsKey($key)) {
                    $numbers[$key] = @{ Kind = 'phrase'; English = ''; File = $null }
                }
                $numbers[$key].Kind = 'phrase'
                if ($base) { $numbers[$key].English = $english }
            }
        } elseif ($rel -match '^ui(-[^/]+)?/(.*/)?([^/]+)\.ogg$') {
            $key = $Matches[3]
            if (-not (Test-Number $key)) { continue }
            if (-not $numbers.ContainsKey($key)) {
                $numbers[$key] = @{ Kind = 'sound'; English = ''; File = $null }
            }
            if (-not $Matches[1] -or -not $numbers[$key].File) { $numbers[$key].File = $rel }
        } elseif ($rel -eq 'ui/style.txt') {
            Read-Style (Get-Text $files[$rel])
        } elseif ($rel -match '^ui/(menu|game)_help\.txt$') {
            foreach ($w in (Get-Text $files[$rel]) -split '\s+') { if (Test-Number $w) { Add-Use $w 'help' } }
        } elseif ($rel -match '^(multi/)?([^/]+)\.txt$' -and $rel -notmatch '^(rules|ai|readme|mods)\.txt$') {
            Read-Map (Get-Text $files[$rel]) "$package/$($rel -replace '[.]txt$', '')"
        }
    }
    $defined[$package] = $numbers
    foreach ($key in $numbers.Keys) {
        if ($numbers[$key].Kind -eq 'sound') { $soundFiles["$package $key"] = $files[$numbers[$key].File] }
    }
}

# The names the code says and plays, in crates/world/src/phrases.rs: the id
# of each phrase, which is its name in snake case, and the sound of each
# cue. The silences are parts of a message of their own, and a style can
# still name them.
$codeNames = @{ 'silence/short' = $true; 'silence/long' = $true }
$phrases = Get-Content -Raw (Join-Path $Repo 'crates/world/src/phrases.rs')
foreach ($m in [regex]::Matches($phrases, '(?m)^\s+([A-Z][A-Za-z0-9]*) = "')) {
    $id = [regex]::Replace($m.Groups[1].Value, '(?<!^)([A-Z])', '_$1').ToLowerInvariant()
    $codeNames[$id] = $true
}
foreach ($m in [regex]::Matches($phrases, 'Cue::[A-Za-z]+ => "([a-z0-9_/]+)"')) {
    $codeNames[$m.Groups[1].Value] = $true
}

# The table as it is: package -> number -> name.
$kept = @{}
if (Test-Path $Table) {
    $package = ''
    foreach ($line in Get-Content $Table) {
        $words = @(($line -split ';')[0].Trim() -split '\s+' | Where-Object { $_ })
        if ($words.Count -eq 0) { continue }
        if ($words[0] -eq 'package') { $package = $words[1]; $kept[$package] = @{}; continue }
        $kept[$package][$words[0]] = $words[1]
    }
}

function Get-Slug([string]$English) {
    $words = @(($English.ToLowerInvariant() -replace "'", '' -replace '[^a-z0-9]+', ' ').Trim() -split ' ' | Where-Object { $_ })
    $slug = ($words | Select-Object -First 6) -join '_'
    if ($slug -match '^[0-9]') { $slug = "number_$slug" }
    $slug
}

# A proposed name for a number without a row.
function Get-Proposal([string]$Number, $Info) {
    if ($Info.Kind -eq 'phrase') {
        $slug = Get-Slug $Info.English
        if ($slug) { return $slug }
        return "phrase_$Number"
    }
    $style = @(if ($uses.ContainsKey($Number)) { $uses[$Number] | Where-Object { $_.StartsWith('style ') } })
    if ($style.Count -eq 0) { return "unnamed/sound_$Number" }
    $def, $attribute = $style[0].Substring(6) -split '\.', 2
    if ($attribute -match '^move_on_(.+)$') { return "footstep/$($Matches[1])" }
    "$attribute/$def"
}

$base = $defined['res']
$out = [System.Collections.Generic.List[string]]::new()
$out.Add('; The name of each number of the original''s files, for each package: the')
$out.Add('; game (res), a map with files of its own, a mod, a campaign.')
$out.Add(';')
$out.Add('; A row is: <number> <name> [unused] ; <comment>')
$out.Add('; A name with a slash is a sound: its path under sounds/, without .ogg.')
$out.Add('; A name without a slash is a phrase. A package lists only the numbers')
$out.Add('; that the game lacks: a number that the game has keeps the game''s name.')
$out.Add('; "unused" marks a number that no style, map, help file or code refers to.')
$out.Add(';')
$out.Add('; Correct a name here by hand. scripts/name-numbers.ps1 adds the missing')
$out.Add('; rows, and writes the unused marks and the comments again. It keeps')
$out.Add('; every name.')
$counts = @{ rows = 0; phrases = 0; sounds = 0; unused = 0; added = 0 }
$copies = [System.Collections.Generic.List[object]]::new()
$gameNames = @{}
foreach ($package in $defined.Keys) {
    $numbers = $defined[$package]
    $own = @($numbers.Keys | Where-Object { $package -eq 'res' -or -not $base.ContainsKey($_) } | Sort-Object { [int]$_ })
    foreach ($key in $numbers.Keys) {
        if ($package -ne 'res' -and $base.ContainsKey($key) -and $numbers[$key].Kind -eq 'sound') {
            $copies.Add(@{ Package = $package; Number = $key; From = 'res' })
        }
    }
    if ($own.Count -eq 0) { continue }
    $names = if ($kept.ContainsKey($package)) { $kept[$package] } else { @{} }
    # A name of the game is not free in another package.
    $taken = if ($package -eq 'res') { @{} } else { $gameNames.Clone() }
    foreach ($key in $own) {
        if (-not $names.ContainsKey($key)) { continue }
        if ($taken.ContainsKey($names[$key])) { throw "$package ${key}: the name '$($names[$key])' is taken." }
        $taken[$names[$key]] = $true
    }
    $out.Add('')
    $out.Add("package $package")
    foreach ($key in $own) {
        $info = $numbers[$key]
        if ($names.ContainsKey($key)) {
            $name = $names[$key]
        } else {
            $name = Get-Proposal $key $info
            $n = 1; $first = $name
            while ($taken.ContainsKey($name)) { $n++; $name = "${first}_$n" }
            $counts.added++
        }
        $taken[$name] = $true
        if ($package -eq 'res') { $gameNames[$name] = $true }
        if ($name -cnotmatch '^[a-z][a-z0-9_]*(/[a-z][a-z0-9_]*)*$') {
            throw "$package ${key}: '$name' is not a name. A name is lower case words, joined by slashes for a sound."
        }
        $isSound = $name.Contains('/')
        if ($isSound -ne ($info.Kind -eq 'sound')) {
            throw "$package ${key}: '$name' is not the name of a $($info.Kind). A sound has a slash and a phrase has none."
        }
        $used = $uses.ContainsKey($key) -or $codeNames.ContainsKey($name)
        $comment = if ($isSound) {
            if ($uses.ContainsKey($key)) {
                (@($uses[$key] | Sort-Object { -not $_.StartsWith('style ') } -Stable) | Select-Object -First 3) -join ', '
            } else { '' }
        } else { $info.English }
        $row = "$key $name"
        if (-not $used) { $row += ' unused'; $counts.unused++ }
        if ($comment) { $row += " ; $comment" }
        $out.Add($row)
        $counts.rows++
        if ($isSound) { $counts.sounds++; $copies.Add(@{ Package = $package; Number = $key; Name = $name }) } else { $counts.phrases++ }
    }
}

$text = ($out -join "`n") + "`n"
[System.IO.File]::WriteAllText($Table, $text, [System.Text.UTF8Encoding]::new($false))
"$($counts.rows) rows: $($counts.phrases) phrases and $($counts.sounds) sounds. $($counts.unused) unused. $($counts.added) added."

if ($CopySoundsTo) {
    $baseNames = @{}
    foreach ($c in $copies) { if ($c.Package -eq 'res' -and $c.ContainsKey('Name')) { $baseNames[$c.Number] = $c.Name } }
    foreach ($c in $copies) {
        $name = if ($c.ContainsKey('Name')) { $c.Name } else { $baseNames[$c.Number] }
        $target = Join-Path $CopySoundsTo "$($c.Package)/$name.ogg"
        New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
        [System.IO.File]::WriteAllBytes($target, (Get-Bytes $soundFiles["$($c.Package) $($c.Number)"]))
    }
    "$($copies.Count) recordings copied to $CopySoundsTo."
}
