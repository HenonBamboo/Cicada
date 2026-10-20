param([switch]$Preview)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent)).TrimEnd('\')
if (-not (Test-Path -LiteralPath (Join-Path $root 'src-tauri/tauri.conf.json') -PathType Leaf) -or
    (Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).name -cne 'cicada') { throw 'This script must run from the Cicada repository scripts directory.' }
$boundary = $root + '\'
$relative = @('dist', 'dist-ssr', 'node_modules', 'releases', 'src-tauri/target', 'src-tauri/gen/schemas', '.acceptance', 'src-tauri/.acceptance', '.playwright-cli', 'output', 'logs', 'src-tauri/logs')
$relative += @(Get-ChildItem -LiteralPath $root -File -Force | Where-Object { $_.Name -like '*.log' } | ForEach-Object Name)
$relative += @(Get-ChildItem -LiteralPath $root -Directory -Force | Where-Object { $_.Name -match '^\.package-[0-9a-f]{32}$' } | ForEach-Object Name)

foreach ($name in $relative) {
    $path = [IO.Path]::GetFullPath((Join-Path $root $name))
    if (-not $path.StartsWith($boundary, [StringComparison]::OrdinalIgnoreCase)) { throw "Refusing path outside repository: $path" }
    if (-not (Test-Path -LiteralPath $path)) { continue }
    # Check the path and every ancestor before enumerating; never traverse a junction.
    $cursor = $path
    while ($cursor.Length -ge $root.Length) {
        $item = Get-Item -LiteralPath $cursor -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing a reparse point: $cursor" }
        if ($cursor -ceq $root) { break }
        $cursor = Split-Path $cursor -Parent
    }
    $pending = [Collections.Generic.Queue[string]]::new()
    if ((Get-Item -LiteralPath $path -Force).PSIsContainer) { $pending.Enqueue($path) }
    while ($pending.Count -gt 0) {
        foreach ($item in Get-ChildItem -LiteralPath $pending.Dequeue() -Force) {
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing to delete a tree containing a reparse point: $($item.FullName)" }
            if ($item.PSIsContainer) { $pending.Enqueue($item.FullName) }
        }
    }
    # A tracked file is never a generated-cleanup target, even if it is inside an allowlisted directory.
    $tracked = & git -C $root ls-files -- $name
    if ($LASTEXITCODE -ne 0 -or $tracked) { throw "Refusing to delete tracked content: $name" }
    if ($Preview) { Write-Output $path } else { Remove-Item -LiteralPath $path -Recurse -Force; Write-Output "Removed: $path" }
}
