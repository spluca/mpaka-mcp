param(
    [Parameter()]
    [string] $Version
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'This installer requires PowerShell 7 or newer.' }
$BaseUrl = 'https://mcp.mpaka.dev'
$PublicKey = @'
-----BEGIN PUBLIC KEY-----
MIIBojANBgkqhkiG9w0BAQEFAAOCAY8AMIIBigKCAYEA1srCWcBjUAB8hvCBTebp
nngaOwUnl+FFBWaU2VQ3WRNj+1tzxjLrRNKc3HEiOUD1TrgufNOoBV4r6/S7O6HW
77rqODhUQ3kixrOFXmzMzDoijsAjKjPknRyB3XkNKbrT/7pqbof5K336HQZz0XfZ
4wuAI2dRPJxsXcXiHGR+KlPJ4EI5/PzEN089qQfshR8yYm62I4/aD7CcrD0idTZS
eqZKki/vDRVsTR42DYFV+yDM4fWmWm2PD0vUMg/JblC9fH3MBEdBIwzth2u203Ih
Fb1UOc+sXLr0ryRimaaRZFFLXWykYNBZN2kt4AToM+rMrUWvN5x7siLwrKUE6veI
nrQM0A5yLBXB0f4AikiRHrcrLojSJR33dzuaeE23suOqyVQmN6DCFhvK+MNR7ObA
m/505u9v4v2XH/wUnNbIQ7bK/G2EK6VKLBKVqATfvLhqLyGBCJ/hehc4qLPEAXSy
NvI1xglPqlySwID/PK89+mYfpqEfLjIs/woEURx4a1FbAgMBAAE=
-----END PUBLIC KEY-----
'@

if (-not $Version) { $Version = $env:MPAKA_MCP_VERSION }
if (-not $Version) {
    $Version = (Invoke-WebRequest -Uri "$BaseUrl/latest" -MaximumRedirection 0 -UseBasicParsing).Content.Trim()
}
if ($Version -notmatch '^v\d+\.\d+\.\d+$') { throw "Invalid release version: $Version" }
$target = 'x86_64-pc-windows-msvc'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    $manifestPath = Join-Path $temporary 'manifest.json'
    $signaturePath = Join-Path $temporary 'manifest.json.sig'
    Invoke-WebRequest -Uri "$BaseUrl/releases/$Version/manifest.json" -OutFile $manifestPath -MaximumRedirection 0 -UseBasicParsing
    Invoke-WebRequest -Uri "$BaseUrl/releases/$Version/manifest.json.sig" -OutFile $signaturePath -MaximumRedirection 0 -UseBasicParsing
    $manifestBytes = [IO.File]::ReadAllBytes($manifestPath)
    $signatureBytes = [IO.File]::ReadAllBytes($signaturePath)
    $rsa = [Security.Cryptography.RSA]::Create()
    try {
        $rsa.ImportFromPem($PublicKey)
        if (-not $rsa.VerifyData($manifestBytes, $signatureBytes, [Security.Cryptography.HashAlgorithmName]::SHA256, [Security.Cryptography.RSASignaturePadding]::Pkcs1)) {
            throw 'Release manifest signature verification failed.'
        }
    } finally { $rsa.Dispose() }

    $manifest = Get-Content -Raw -Path $manifestPath | ConvertFrom-Json
    if ($manifest.version -ne $Version) { throw 'Release manifest version does not match.' }
    $artifact = @($manifest.artifacts | Where-Object target -eq $target)
    if ($artifact.Count -ne 1) { throw "Manifest has no unique artifact for $target." }
    $item = $artifact[0]
    $expectedName = "mpaka-mcp-$Version-$target.zip"
    if ($item.filename -ne $expectedName) { throw 'Manifest selected an unexpected archive.' }
    $archivePath = Join-Path $temporary $item.filename
    Invoke-WebRequest -Uri "$BaseUrl/releases/$Version/$($item.filename)" -OutFile $archivePath -MaximumRedirection 0 -UseBasicParsing
    if ((Get-Item $archivePath).Length -ne [long]$item.size) { throw 'Downloaded archive size does not match manifest.' }
    if ((Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant() -ne $item.sha256) { throw 'Downloaded archive SHA-256 does not match manifest.' }

    $extract = Join-Path $temporary 'extract'
    Expand-Archive -LiteralPath $archivePath -DestinationPath $extract
    $entries = @(Get-ChildItem -LiteralPath $extract -Force)
    if ($entries.Count -ne 1 -or $entries[0].Name -ne 'mpaka-mcp.exe' -or $entries[0].PSIsContainer) { throw 'Archive contains unexpected files.' }
    $installDir = Join-Path $env:USERPROFILE '.local\bin'
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $destination = Join-Path $installDir 'mpaka-mcp.exe'
    Copy-Item -LiteralPath $entries[0].FullName -Destination $destination -Force
    Write-Output "Installed mpaka-mcp $Version for $target at $destination"
    if (($env:PATH -split ';') -notcontains $installDir) { Write-Output "Add $installDir to PATH to run mpaka-mcp." }
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
