param(
    [Parameter(Mandatory)][string]$ManifestPath,
    [Parameter(Mandatory)][string]$SignaturePath,
    [string]$PrivateKeyPath = $env:CICADA_UPDATE_PRIVATE_KEY_PATH
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$privateKeyPem = $null
if (-not $PSBoundParameters.ContainsKey('PrivateKeyPath')) {
    $privateKeyPem = $env:CICADA_UPDATE_PRIVATE_KEY
    if ([string]::IsNullOrWhiteSpace($privateKeyPem)) { $privateKeyPem = [Environment]::GetEnvironmentVariable('CICADA_UPDATE_PRIVATE_KEY', 'User') }
}
if ([string]::IsNullOrWhiteSpace($privateKeyPem)) {
    if ([string]::IsNullOrWhiteSpace($PrivateKeyPath)) { $PrivateKeyPath = [Environment]::GetEnvironmentVariable('CICADA_UPDATE_PRIVATE_KEY_PATH', 'User') }
    if ([string]::IsNullOrWhiteSpace($PrivateKeyPath) -or -not (Test-Path -LiteralPath $PrivateKeyPath -PathType Leaf)) {
        throw 'Set CICADA_UPDATE_PRIVATE_KEY to the release PEM text, or CICADA_UPDATE_PRIVATE_KEY_PATH to its file outside the repository.'
    }
}
$appRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$openssl = (Get-Command openssl.exe -ErrorAction Stop).Source
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('cicada-sign-' + [guid]::NewGuid().ToString('N'))
if ([IO.Path]::GetFullPath($temporary).StartsWith($appRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Signing temporary files must be outside the repository.' }
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    if (-not [string]::IsNullOrWhiteSpace($privateKeyPem)) {
        if ($privateKeyPem.Length -gt 4096 -or $privateKeyPem -notmatch '\A\s*-----BEGIN PRIVATE KEY-----\s+[A-Za-z0-9+/=\r\n]+-----END PRIVATE KEY-----\s*\z') { throw 'CICADA_UPDATE_PRIVATE_KEY must contain an unencrypted PKCS8 PEM key.' }
        $account = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        & icacls.exe $temporary /inheritance:r /grant:r "${account}:(OI)(CI)F" 'SYSTEM:(OI)(CI)F' | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Unable to protect the temporary signing key.' }
        $resolvedKey = Join-Path $temporary 'signing-key.pem'
        [IO.File]::WriteAllText($resolvedKey, $privateKeyPem, [Text.UTF8Encoding]::new($false))
    } else {
        $resolvedKey = (Resolve-Path -LiteralPath $PrivateKeyPath).Path
        if ($resolvedKey.StartsWith($appRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'The release private key must be outside the repository.' }
    }
    $publicDer = Join-Path $temporary 'public.der'
    & $openssl pkey -in $resolvedKey -pubout -outform DER -out $publicDer 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Unable to read the Ed25519 release key.' }
    $bytes = [IO.File]::ReadAllBytes($publicDer)
    if ($bytes.Length -ne 44 -or [BitConverter]::ToString($bytes[0..11]).Replace('-', '').ToLowerInvariant() -ne '302a300506032b6570032100') { throw 'The release key must be Ed25519.' }
    $expected = (Get-Content -LiteralPath (Join-Path $appRoot 'src-tauri/src/updater/public-key.txt') -Raw).Trim()
    if ([Convert]::ToBase64String($bytes[12..43]) -cne $expected) { throw 'Release private key does not match the public key embedded in the application.' }
    $rawSignature = Join-Path $temporary 'signature.bin'
    & $openssl pkeyutl -sign -rawin -inkey $resolvedKey -in $ManifestPath -out $rawSignature 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Manifest signing failed.' }
    & $openssl pkeyutl -verify -rawin -pubin -keyform DER -inkey $publicDer -in $ManifestPath -sigfile $rawSignature 1>$null 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Manifest signature verification failed.' }
    $signature = [IO.File]::ReadAllBytes($rawSignature)
    if ($signature.Length -ne 64) { throw 'Invalid Ed25519 signature length.' }
    [IO.File]::WriteAllText([IO.Path]::GetFullPath($SignaturePath), [Convert]::ToBase64String($signature) + "`n", [Text.UTF8Encoding]::new($false))
} finally {
    $privateKeyPem = $null
    foreach ($name in @('signing-key.pem', 'public.der', 'signature.bin')) {
        $file = Join-Path $temporary $name
        if (Test-Path -LiteralPath $file -PathType Leaf) { Remove-Item -LiteralPath $file -Force }
    }
    Remove-Item -LiteralPath $temporary -Force
}
