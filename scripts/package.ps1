param(
    [string]$OutputDirectory = (Join-Path ([IO.Path]::GetTempPath()) ('Cicada-release-' + [guid]::NewGuid().ToString('N'))),
    [string]$NotesFile,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$appRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$previousLocation = Get-Location
$staging = $null
if ($NotesFile) { $NotesFile = (Resolve-Path -LiteralPath $NotesFile).Path }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)

function Read-CargoVersion([string]$Path) {
    $block = [regex]::Match((Get-Content -LiteralPath $Path -Raw -Encoding UTF8), '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
    $version = [regex]::Match($block.Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $version.Success) { throw 'Cargo package version is missing.' }
    return $version.Groups[1].Value
}

try {
    if (($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) -or -not [Environment]::Is64BitOperatingSystem) { throw 'Windows x64 is required.' }
    Set-Location -LiteralPath $appRoot
    $package = Get-Content -LiteralPath 'package.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $tauri = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $version = [string]$package.version
    if ($version -notmatch '^\d+\.\d+\.\d+$' -or $version -cne (Read-CargoVersion 'src-tauri/Cargo.toml') -or $version -cne $tauri.version) { throw 'Release versions must match and must be stable semantic versions.' }
    if ($tauri.identifier -cne 'com.cicada.app' -or $tauri.bundle.active) { throw 'The direct EXE configuration must keep identifier com.cicada.app and bundle.active=false.' }
    $rustVersion = & rustc -vV
    if ($LASTEXITCODE -ne 0 -or -not ($rustVersion -match '(?m)^host: x86_64-pc-windows-msvc$')) { throw 'The active Rust toolchain must target x86_64-pc-windows-msvc.' }
    $documents = @('README.md', 'LICENSE', 'PRIVACY.md', 'THIRD_PARTY_NOTICES.txt', 'CHANGELOG.md')
    foreach ($name in $documents) {
        if (-not (Test-Path -LiteralPath $name -PathType Leaf)) { throw "Required release document missing: $name" }
    }
    $notices = Get-Content -LiteralPath 'THIRD_PARTY_NOTICES.txt' -Raw -Encoding UTF8
    foreach ($entry in @{ 'package-lock.json' = 'package-lock.json'; 'Cargo.lock' = 'src-tauri/Cargo.lock' }.GetEnumerator()) {
        $hash = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash
        if (-not $notices.Contains("$($entry.Key) SHA-256: $hash")) { throw "THIRD_PARTY_NOTICES.txt is stale for $($entry.Key)." }
    }
    if ($NotesFile) {
        $notes = (Get-Content -LiteralPath $NotesFile -Raw -Encoding UTF8).Trim()
    } else {
        $changelog = Get-Content -LiteralPath 'CHANGELOG.md' -Raw -Encoding UTF8
        $section = [regex]::Match($changelog, '(?ms)^##\s+\[?v?' + [regex]::Escape($version) + '\]?(?:[^\r\n]*)\r?\n(.*?)(?=^##\s|\z)')
        if (-not $section.Success) { throw "CHANGELOG.md is missing the $version section." }
        $notes = $section.Groups[1].Value.Trim()
    }
    if (-not $notes -or $notes.Length -gt 6000) { throw 'Release notes must contain 1 to 6000 characters.' }
    $output = [IO.Path]::GetFullPath($OutputDirectory)
    if (Test-Path -LiteralPath $output) {
        if ((Get-ChildItem -LiteralPath $output -Force | Measure-Object).Count -gt 0) { throw 'OutputDirectory must be empty; existing release files will not be overwritten.' }
    } else { New-Item -ItemType Directory -Path $output | Out-Null }
    $staging = Join-Path $output ('.package-' + [guid]::NewGuid().ToString('N'))
    $portable = Join-Path $staging 'portable'
    New-Item -ItemType Directory -Path $portable -Force | Out-Null
    if (-not $SkipBuild) {
        & npm.cmd run tauri -- build --ci --no-bundle -- --locked
        if ($LASTEXITCODE -ne 0) { throw "Tauri build failed: $LASTEXITCODE" }
    }
    $built = Join-Path $appRoot 'src-tauri/target/release/cicada.exe'
    if (-not (Test-Path -LiteralPath $built -PathType Leaf)) { throw 'Release executable was not generated.' }
    $exeName = "Cicada-$version-windows-x64.exe"
    $zipName = "Cicada-$version-windows-x64-portable.zip"
    $exe = Join-Path $output $exeName
    Copy-Item -LiteralPath $built -Destination $exe
    Copy-Item -LiteralPath $built -Destination (Join-Path $portable 'cicada.exe')
    foreach ($name in $documents) { Copy-Item -LiteralPath (Join-Path $appRoot $name) -Destination (Join-Path $portable $name) }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($portable, (Join-Path $output $zipName), [IO.Compression.CompressionLevel]::Optimal, $false)
    $manifest = [ordered]@{
        schemaVersion = 1
        version = $version
        tag = "v$version"
        asset = $exeName
        size = (Get-Item -LiteralPath $exe).Length
        sha256 = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        publishedAt = [DateTimeOffset]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
        notes = $notes
    }
    $manifestPath = Join-Path $output 'latest.json'
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 5) + "`n", [Text.UTF8Encoding]::new($false))
    & (Join-Path $PSScriptRoot 'sign-update.ps1') -ManifestPath $manifestPath -SignaturePath (Join-Path $output 'latest.json.sig')
    $checksums = foreach ($name in @($exeName, $zipName, 'latest.json', 'latest.json.sig')) {
        $hash = (Get-FileHash -LiteralPath (Join-Path $output $name) -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $name"
    }
    [IO.File]::WriteAllText((Join-Path $output 'SHA256SUMS'), ($checksums -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    Get-ChildItem -LiteralPath $output -File | Select-Object Name, Length
    Write-Host "Release directory: $output"
} finally {
    Set-Location -LiteralPath $previousLocation.Path
    if ($staging -and (Test-Path -LiteralPath $staging)) {
        $resolved = [IO.Path]::GetFullPath($staging)
        $boundary = [IO.Path]::GetFullPath($OutputDirectory).TrimEnd('\') + '\'
        if (-not $resolved.StartsWith($boundary, [StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing to remove staging outside the release output.' }
        $reparse = @(Get-Item -LiteralPath $staging -Force; Get-ChildItem -LiteralPath $staging -Force -Recurse) | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }
        if ($reparse) { throw 'Refusing to remove staging containing a reparse point.' }
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}
