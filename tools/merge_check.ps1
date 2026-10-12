# Record, then re-check, everything the analyser writes, across the merge into
# TelegramTools.
#
#   tools\merge_check.ps1 -Exe <TelegramAnalyser.exe or TelegramTools.exe> -Out <dir>
#       [-Args0 analyse]       a leading subcommand, once the exe needs one
#
# Writes <dir>\MANIFEST.txt: one sha256 per output, plus model_hash's lines.
# Run it before the merge and after, into two folders, and compare the two
# manifests. They should match line for line.
#
# **Why this exists alongside `save.bat baseline`:** the baseline's two folder
# corpora (UA KOLAB TELEGRAM, KRGM) were not mounted on the day of the merge,
# and the baseline never covered the digest or the database reader anyway.
# The merge threatens exactly those two. `serde_json`'s `preserve_order` gets
# switched on for every crate, and `tga-db` moves from rusqlite to libsql. So
# this records what was on hand:
#
#   - the exporter's own reference\ cut of UA KOLAB, which is our folder layout
#     (result.json per topic at the root). It is copied to one fixed scratch
#     folder first, because --digest writes analysis\ beside the export, and the
#     reference tree is the parity oracle, which must not be written into.
#     **Fixed, not inside <dir>:** the export's path is in the stats dump and
#     the report (`root`), so a copy under each run's own <dir> made every
#     before/after pair differ by that one string.
#   - three databases on L:, small, medium and large. They get the report and
#     stats dump only. A digest would go into the export folder on L:, which
#     is somebody's archive.
#
# Everything under <dir> is verbatim chat content. Keep <dir> under a
# gitignored folder (baseline\ is).

param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Out,
    [string] $Args0 = "",
    [string] $Reference = "$PSScriptRoot\..\reference",
    [string] $Cargo = "$PSScriptRoot\.."
)

$ErrorActionPreference = "Stop"
# Pinned, so two runs on different days write the same notes stamp.
$stamp = "2026-10-12"
$dbs = @(
    "L:\9 telegram export\UA KOLAB\UA KOLAB.sqlite",
    "L:\9 telegram export\Admini\Admini.sqlite",
    "L:\9 telegram export\admini x koord vol2\admini x koord vol2.sqlite"
)

New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$pre = @(); if ($Args0) { $pre = @($Args0) }

function Run([string[]] $a) {
    & $Exe @pre @a --quiet --stamp $stamp | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "$Exe $($pre + $a) exited $LASTEXITCODE" }
}

# -- the folder layout, with the digest ------------------------------------
$folder = Join-Path $env:TEMP "tt-merge-check\ua-folder"
if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
New-Item -ItemType Directory -Force (Split-Path $folder) | Out-Null
Copy-Item -Recurse $Reference $folder
Get-ChildItem $folder -Recurse -Filter "messages*.html" | Remove-Item
Remove-Item (Join-Path $folder "MANIFEST.txt") -ErrorAction SilentlyContinue
Run @($folder, "--out", "$Out\ua-folder.report.html", "--stats", "$Out\ua-folder.stats.json", "--digest")
Run @($folder, "--out", "$Out\ua-folder-handles.report.html", "--stats", "$Out\ua-folder-handles.stats.json", "--handles")
Copy-Item -Recurse -Force (Join-Path $folder "analysis") (Join-Path $Out "ua-folder-analysis")
Remove-Item -Recurse -Force (Split-Path $folder)

# -- the databases ----------------------------------------------------------
$i = 0
foreach ($db in $dbs) {
    $i++
    Run @($db, "--out", "$Out\db$i.report.html", "--stats", "$Out\db$i.stats.json")
}

# -- the manifest -----------------------------------------------------------
$lines = Get-ChildItem $Out -Recurse -File |
    Where-Object { $_.Name -ne "MANIFEST.txt" -and $_.Name -notlike "result.json" } |
    Sort-Object FullName |
    ForEach-Object { "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.FullName.Substring($Out.Length + 1) }

Push-Location $Cargo
try {
    $model = cargo run --quiet --release -p tga-db --example model_hash -- @dbs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "model_hash failed: $model" }
} finally { Pop-Location }

($lines + "" + "model_hash:" + $model) | Set-Content -Encoding utf8 "$Out\MANIFEST.txt"
Get-Content "$Out\MANIFEST.txt"
