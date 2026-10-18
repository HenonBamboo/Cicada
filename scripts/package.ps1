$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$appRoot = Split-Path $PSScriptRoot -Parent
$repoRoot = $appRoot
$previousLocation = Get-Location
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
$packageTemp = $null
$output = $null

function Get-CargoPackageVersion {
    param([Parameter(Mandatory)][string]$Path)

    $content = Get-Content -LiteralPath $Path -Raw
    $packageBlock = [regex]::Match($content, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
    if (-not $packageBlock.Success) {
        throw "Cargo.toml does not contain a [package] section: $Path"
    }

    $versionMatch = [regex]::Match($packageBlock.Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"\s*$')
    if (-not $versionMatch.Success) {
        throw "Cargo.toml [package] does not declare version: $Path"
    }
    return $versionMatch.Groups[1].Value
}

function Install-AtomicFile {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination
    )

    if (Test-Path -LiteralPath $Destination -PathType Leaf) {
        $backup = Join-Path (Split-Path $Source -Parent) ([System.IO.Path]::GetFileName($Destination) + '.previous')
        [System.IO.File]::Replace($Source, $Destination, $backup, $true)
        Remove-Item -LiteralPath $backup -Force
    } else {
        [System.IO.File]::Move($Source, $Destination)
    }
}

try {
    if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
        throw 'This release script only supports Windows.'
    }
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'Cicada 0.1.0 release packaging requires Windows x64.'
    }

    $rustVersion = & rustc -vV
    if ($LASTEXITCODE -ne 0 -or -not ($rustVersion -match '(?m)^host: x86_64-pc-windows-msvc$')) {
        throw 'The active Rust toolchain must target x86_64-pc-windows-msvc.'
    }

    Set-Location -LiteralPath $appRoot

    $packageJsonPath = Join-Path $appRoot 'package.json'
    $cargoTomlPath = Join-Path $appRoot 'src-tauri/Cargo.toml'
    $tauriConfigPath = Join-Path $appRoot 'src-tauri/tauri.conf.json'

    $packageJson = Get-Content -LiteralPath $packageJsonPath -Raw | ConvertFrom-Json
    $tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
    $versions = [ordered]@{
        'package.json' = [string]$packageJson.version
        'Cargo.toml' = Get-CargoPackageVersion -Path $cargoTomlPath
        'tauri.conf.json' = [string]$tauriConfig.version
    }
    $uniqueVersions = @($versions.Values | Select-Object -Unique)
    if ($uniqueVersions.Count -ne 1 -or [string]::IsNullOrWhiteSpace($uniqueVersions[0])) {
        $details = ($versions.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '
        throw "Release versions do not match: $details"
    }
    $version = $uniqueVersions[0]

    if ([string]$tauriConfig.identifier -ne 'com.cicada.app') {
        throw "Unexpected application identifier: $($tauriConfig.identifier)"
    }
    if ([bool]$tauriConfig.bundle.active) {
        throw 'tauri.conf.json bundle.active must remain false for the direct EXE release.'
    }

    $releaseDocuments = @(
        'README.md',
        'LICENSE',
        'PRIVACY.md',
        'THIRD_PARTY_NOTICES.txt',
        'CHANGELOG.md'
    )
    foreach ($name in $releaseDocuments) {
        $path = Join-Path $repoRoot $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Required release document is missing: $path"
        }
    }

    $noticesPath = Join-Path $repoRoot 'THIRD_PARTY_NOTICES.txt'
    $noticesContent = Get-Content -LiteralPath $noticesPath -Raw
    $dependencyLocks = [ordered]@{
        'package-lock.json' = Join-Path $appRoot 'package-lock.json'
        'Cargo.lock' = Join-Path $appRoot 'src-tauri/Cargo.lock'
    }
    foreach ($entry in $dependencyLocks.GetEnumerator()) {
        $hash = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash
        $fingerprint = "$($entry.Key) SHA-256: $hash"
        if (-not $noticesContent.Contains($fingerprint)) {
            throw "THIRD_PARTY_NOTICES.txt is stale for $($entry.Key); regenerate it before release."
        }
    }

    $output = Join-Path $appRoot 'releases'
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    # Keep temporary release files on the same volume as the final artifacts so
    # File.Replace can install each completed file atomically.
    $packageTemp = Join-Path $output ('.package-' + [guid]::NewGuid().ToString('N'))
    $stage = Join-Path $packageTemp 'portable'
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    $env:TEMP = $packageTemp
    $env:TMP = $packageTemp

    & npm.cmd run tauri -- build --ci --no-bundle -- --offline
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri build failed: $LASTEXITCODE"
    }

    $builtExecutable = Join-Path $appRoot 'src-tauri/target/release/cicada.exe'
    if (-not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) {
        throw "Release executable was not generated: $builtExecutable"
    }

    $stagedExecutable = Join-Path $stage 'cicada.exe'
    Copy-Item -LiteralPath $builtExecutable -Destination $stagedExecutable
    foreach ($name in $releaseDocuments) {
        Copy-Item -LiteralPath (Join-Path $repoRoot $name) -Destination (Join-Path $stage $name)
    }

    $portableName = "Cicada-$version-windows-x64-portable.zip"
    $temporaryPortable = Join-Path $packageTemp $portableName

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory(
        $stage,
        $temporaryPortable,
        [System.IO.Compression.CompressionLevel]::Optimal,
        $false
    )

    $exeHash = (Get-FileHash -LiteralPath $stagedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $zipHash = (Get-FileHash -LiteralPath $temporaryPortable -Algorithm SHA256).Hash.ToLowerInvariant()
    $temporaryChecksums = Join-Path $packageTemp 'SHA256SUMS.txt'
    $checksumContent = "$exeHash  cicada.exe`n$zipHash  $portableName`n"
    [System.IO.File]::WriteAllText(
        $temporaryChecksums,
        $checksumContent,
        [System.Text.UTF8Encoding]::new($false)
    )

    $temporaryExecutable = Join-Path $packageTemp 'cicada.exe'
    Copy-Item -LiteralPath $stagedExecutable -Destination $temporaryExecutable

    $executable = Join-Path $output 'cicada.exe'
    $portable = Join-Path $output $portableName
    $checksums = Join-Path $output 'SHA256SUMS.txt'
    Install-AtomicFile -Source $temporaryExecutable -Destination $executable
    Install-AtomicFile -Source $temporaryPortable -Destination $portable
    Install-AtomicFile -Source $temporaryChecksums -Destination $checksums

    # The current release is EXE/portable-only. Remove only the explicitly named
    # installer artifact from the same version after all new artifacts succeed.
    $obsoleteInstaller = Join-Path $output "Cicada_${version}_x64-setup.exe"
    if (Test-Path -LiteralPath $obsoleteInstaller -PathType Leaf) {
        Remove-Item -LiteralPath $obsoleteInstaller -Force
    }

    Get-Item -LiteralPath $executable, $portable, $checksums |
        Select-Object Name, Length, LastWriteTime
} finally {
    if ($null -eq $previousTemp) { Remove-Item Env:TEMP -ErrorAction SilentlyContinue } else { $env:TEMP = $previousTemp }
    if ($null -eq $previousTmp) { Remove-Item Env:TMP -ErrorAction SilentlyContinue } else { $env:TMP = $previousTmp }
    Set-Location -LiteralPath $previousLocation.Path

    if ($packageTemp -and (Test-Path -LiteralPath $packageTemp)) {
        $resolvedTemp = [System.IO.Path]::GetFullPath($packageTemp)
        $resolvedOutput = [System.IO.Path]::GetFullPath($output).TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
        if (-not $resolvedTemp.StartsWith($resolvedOutput, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove temporary path outside the release directory: $resolvedTemp"
        }
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
    }
}
