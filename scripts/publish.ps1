param(
    [Parameter(Mandatory)][string]$ArtifactDirectory,
    [Parameter(Mandatory)][string]$NotesFile
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$artifacts = (Resolve-Path -LiteralPath $ArtifactDirectory).Path
$manifest = Get-Content -LiteralPath (Join-Path $artifacts 'latest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$manifest.version
$tag = [string]$manifest.tag
$releaseName = '{0}{1} {2}' -f [char]0x77E5, [char]0x4E86, $version
if ($version -notmatch '^\d+\.\d+\.\d+$' -or $tag -cne "v$version") { throw 'Invalid stable version/tag.' }
$package = Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$tauri = Get-Content -LiteralPath (Join-Path $root 'src-tauri/tauri.conf.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$cargoText = Get-Content -LiteralPath (Join-Path $root 'src-tauri/Cargo.toml') -Raw -Encoding UTF8
$cargoPackage = [regex]::Match($cargoText, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)')
$cargoVersion = [regex]::Match($cargoPackage.Groups[1].Value, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($version -cne $package.version -or $version -cne $tauri.version -or $version -cne $cargoVersion) { throw 'Manifest and source versions must match.' }
$assets = @("Cicada-$version-windows-x64.exe", "Cicada-$version-windows-x64-portable.zip", 'SHA256SUMS', 'latest.json', 'latest.json.sig')
$actualNames = @(Get-ChildItem -LiteralPath $artifacts -Force | ForEach-Object Name | Sort-Object)
if (Compare-Object ($assets | Sort-Object) $actualNames) { throw 'ArtifactDirectory must contain exactly the five release assets.' }
$checksums = Get-Content -LiteralPath (Join-Path $artifacts 'SHA256SUMS')
foreach ($name in $assets | Where-Object { $_ -cne 'SHA256SUMS' }) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $artifacts $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    if (@($checksums | Where-Object { $_ -ceq "$hash  $name" }).Count -ne 1) { throw "Checksum mismatch: $name" }
}
if ($manifest.asset -cne $assets[0] -or $manifest.sha256 -cne (Get-FileHash -LiteralPath (Join-Path $artifacts $assets[0]) -Algorithm SHA256).Hash.ToLowerInvariant() -or $manifest.size -ne (Get-Item -LiteralPath (Join-Path $artifacts $assets[0])).Length) { throw 'Manifest does not describe the packaged EXE.' }
$openssl = (Get-Command openssl.exe -ErrorAction Stop).Source
$publicKey = [Convert]::FromBase64String((Get-Content -LiteralPath (Join-Path $root 'src-tauri/src/updater/public-key.txt') -Raw).Trim())
$signature = [Convert]::FromBase64String((Get-Content -LiteralPath (Join-Path $artifacts 'latest.json.sig') -Raw).Trim())
if ($publicKey.Length -ne 32 -or $signature.Length -ne 64) { throw 'Invalid Ed25519 public key or signature.' }
$keyFile = [IO.Path]::GetTempFileName()
$signatureFile = [IO.Path]::GetTempFileName()
try {
    [byte[]]$derHeader = @(0x30,0x2a,0x30,0x05,0x06,0x03,0x2b,0x65,0x70,0x03,0x21,0x00)
    [IO.File]::WriteAllBytes($keyFile, [byte[]]($derHeader + $publicKey))
    [IO.File]::WriteAllBytes($signatureFile, $signature)
    & $openssl pkeyutl -verify -rawin -pubin -keyform DER -inkey $keyFile -in (Join-Path $artifacts 'latest.json') -sigfile $signatureFile 1>$null 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Release manifest signature does not match the embedded public key.' }
} finally {
    Remove-Item -LiteralPath $keyFile, $signatureFile -Force
}
if ([string]::IsNullOrWhiteSpace($env:GITCODE_TOKEN)) { throw 'Set GITCODE_TOKEN in the publishing process environment; it is never embedded in the application.' }
$notes = Get-Content -LiteralPath $NotesFile -Raw -Encoding UTF8
$dirty = & git -C $root status --porcelain
if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'Commit release sources and clean intermediate files before publishing.' }
$intermediates = @(& (Join-Path $PSScriptRoot 'clean-release.ps1') -Preview)
if ($intermediates.Count -ne 0) { throw 'Remove the allowlisted intermediate directories with clean-release.ps1 before uploading release content.' }
$head = & git -C $root rev-parse HEAD
$tagCommit = & git -C $root rev-parse "$tag^{commit}"
if ($LASTEXITCODE -ne 0 -or $head -cne $tagCommit) { throw 'The release tag must resolve to the current commit.' }
foreach ($remote in @('origin', 'gitcode')) {
    $url = & git -C $root remote get-url $remote
    $expected = if ($remote -ceq 'origin') { 'git@github.com:HenonBamboo/Cicada.git' } else { 'git@gitcode.com:HenonBamboo_YYJ/Cicada.git' }
    if ($LASTEXITCODE -ne 0 -or $url -cne $expected) { throw "Unexpected $remote remote; expected $expected" }
}
& gh auth status 1>$null 2>$null
if ($LASTEXITCODE -ne 0) { throw 'GitHub CLI authentication is required.' }

$api = 'https://api.gitcode.com/api/v5/repos/HenonBamboo_YYJ/Cicada'
function Invoke-GitCode([string]$Method, [string]$Path, [object]$Body = $null) {
    # Token remains in an HTTP header, outside URLs, logs, assets and source files.
    $uri = $api + $Path
    $authHeaders = @{ Authorization = 'Bearer ' + $env:GITCODE_TOKEN }
    try {
        if ($null -ne $Body) { return Invoke-RestMethod -Method $Method -Uri $uri -Headers $authHeaders -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 8 -Compress))) }
        return Invoke-RestMethod -Method $Method -Uri $uri -Headers $authHeaders
    } catch {
        $status = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { 0 }
        if ($status -eq 400 -and $Method -ieq 'Get' -and $Path -match '^/releases/tags/[^/?]+$') {
            $details = $null
            if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
                try { $details = $_.ErrorDetails.Message | ConvertFrom-Json } catch { $details = $null }
            }
            $missingRelease = '{0}{1}{2} release' -f [char]0x672A, [char]0x627E, [char]0x5230
            if ($null -ne $details -and $details.PSObject.Properties['error_message'] -and ([string]$details.error_message).Trim() -ceq $missingRelease) { return $null }
        }
        throw "GitCode $Method request failed (HTTP $status); credentials and response details omitted."
    }
}
# Check the token before either remote is changed.
$null = Invoke-GitCode 'Get' ''
foreach ($remote in @('origin', 'gitcode')) {
    & git -C $root push $remote 'HEAD:main' "refs/tags/$tag"
    if ($LASTEXITCODE -ne 0) { throw "Push to $remote failed; no force push was attempted." }
    $published = @(& git -C $root ls-remote $remote 'refs/heads/main' "refs/tags/$tag" "refs/tags/$tag^{}")
    if ($LASTEXITCODE -ne 0) { throw "Remote ref query failed: $remote" }
    $mainRef = @($published | Where-Object { $_ -match '\s+refs/heads/main$' })
    $tagRef = @($published | Where-Object { $_ -match ('\s+refs/tags/' + [regex]::Escape($tag) + '\^\{\}$') })
    if ($tagRef.Count -eq 0) { $tagRef = @($published | Where-Object { $_ -match ('\s+refs/tags/' + [regex]::Escape($tag) + '$') }) }
    if ($mainRef.Count -ne 1 -or $tagRef.Count -ne 1 -or ($mainRef[0] -split '\s+')[0] -cne $head -or ($tagRef[0] -split '\s+')[0] -cne $head) { throw "Remote main/tag commit verification failed: $remote" }
}

$nativePreference = $ErrorActionPreference
try {
    # Windows PowerShell 5.1 将预期的“release not found” stderr 也视作错误记录。
    $ErrorActionPreference = 'Continue'
    $githubJson = & gh release view $tag --repo HenonBamboo/Cicada --json assets 2>$null
    $githubStatus = $LASTEXITCODE
} finally { $ErrorActionPreference = $nativePreference }
if ($githubStatus -ne 0) {
    & gh release create $tag --repo HenonBamboo/Cicada --verify-tag --title $releaseName --notes-file $NotesFile
    if ($LASTEXITCODE -ne 0) { throw 'GitHub release creation failed.' }
} else {
    $existing = @((($githubJson -join "`n") | ConvertFrom-Json).assets | ForEach-Object Name)
    if (@($existing | Where-Object { $assets -cnotcontains $_ }).Count -gt 0) { throw 'Existing GitHub release contains unrelated assets; no files were replaced.' }
}
$paths = @($assets | ForEach-Object { Join-Path $artifacts $_ })
& gh release upload $tag @paths --repo HenonBamboo/Cicada --clobber
if ($LASTEXITCODE -ne 0) { throw 'GitHub release upload failed.' }
$release = $null
try { $release = Invoke-GitCode 'Get' "/releases/tags/$tag" } catch { if ($_ -notmatch 'HTTP 404') { throw } }
if ($null -eq $release) {
    $release = Invoke-GitCode 'Post' '/releases' @{ tag_name = $tag; target_commitish = $head; name = $releaseName; body = $notes; release_status = 'latest' }
}
function Assert-GitCodeAssets([object]$Release, [bool]$RequireAll) {
    $sourcePattern = '^Cicada-v?' + [regex]::Escape($version) + '\.(zip|tar|tar\.gz|tar\.bz2)$'
    $tagSourcePattern = '^' + [regex]::Escape($tag) + '\.(zip|tar|tar\.gz|tar\.bz2)$'
    $names = @($Release.assets | Where-Object { -not ($_.type -ceq 'source' -and ($_.name -match $sourcePattern -or $_.name -match $tagSourcePattern)) } | ForEach-Object name)
    if (@($names | Where-Object { $assets -cnotcontains $_ }).Count -gt 0) { throw 'GitCode release contains unrelated assets; no files were replaced.' }
    if ($RequireAll -and (Compare-Object ($assets | Sort-Object) ($names | Sort-Object))) { throw 'GitCode release does not contain exactly the five expected attachments.' }
}
Assert-GitCodeAssets $release $false
foreach ($name in $assets) {
    $upload = Invoke-GitCode 'Get' ("/releases/$tag/upload_url?file_name=" + [uri]::EscapeDataString($name))
    $uploadUri = [uri]$upload.url
    if ($uploadUri.Scheme -cne 'https' -or -not $uploadUri.IsAbsoluteUri -or $uploadUri.UserInfo) { throw 'GitCode returned an invalid upload address.' }
    $headers = @{}
    foreach ($property in $upload.headers.PSObject.Properties) { $headers[$property.Name] = [string]$property.Value }
    try { $null = Invoke-WebRequest -Method Put -Uri $uploadUri -Headers $headers -InFile (Join-Path $artifacts $name) } catch { throw "GitCode attachment upload failed: $name (details omitted)." }
}

$verification = Join-Path ([IO.Path]::GetTempPath()) ('Cicada-release-verify-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $verification | Out-Null
try {
    $githubJson = & gh release view $tag --repo HenonBamboo/Cicada --json assets
    if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect GitHub release attachments.' }
    $githubNames = @((($githubJson -join "`n") | ConvertFrom-Json).assets | ForEach-Object Name)
    if (Compare-Object ($assets | Sort-Object) ($githubNames | Sort-Object)) { throw 'GitHub release does not contain exactly the five expected attachments.' }
    $gitcodeRelease = Invoke-GitCode 'Get' "/releases/tags/$tag"
    Assert-GitCodeAssets $gitcodeRelease $true
    foreach ($name in $assets) {
        $download = Join-Path $verification $name
        & gh release download $tag --repo HenonBamboo/Cicada --pattern $name --dir $verification --clobber
        if ($LASTEXITCODE -ne 0 -or (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -cne (Get-FileHash -LiteralPath (Join-Path $artifacts $name) -Algorithm SHA256).Hash) { throw "GitHub checksum verification failed: $name" }
        Remove-Item -LiteralPath $download -Force
        # Public, unauthenticated URL is exactly what the application uses.
        try { Invoke-WebRequest -Uri "$api/releases/$tag/attach_files/$name/download" -OutFile $download | Out-Null } catch { throw "Public GitCode download failed: $name (details omitted)." }
        if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -cne (Get-FileHash -LiteralPath (Join-Path $artifacts $name) -Algorithm SHA256).Hash) { throw "GitCode checksum verification failed: $name" }
        Remove-Item -LiteralPath $download -Force
    }
    $latest = Invoke-RestMethod -Uri "$api/releases/latest?type=latest"
    if ($latest.tag_name -cne $tag) { throw 'GitCode latest release does not point to the newly published version.' }
    Write-Output "Published and verified $tag at the same commit and SHA-256 assets on GitHub and GitCode."
} finally {
    foreach ($name in $assets) {
        $download = Join-Path $verification $name
        if (Test-Path -LiteralPath $download -PathType Leaf) { Remove-Item -LiteralPath $download -Force }
    }
    Remove-Item -LiteralPath $verification -Force
}
