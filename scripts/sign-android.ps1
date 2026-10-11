param(
    [Parameter(Mandatory)][string]$UnsignedApk,
    [Parameter(Mandatory)][string]$OutputApk,
    [string]$SigningRoot = "$env:LOCALAPPDATA/art-term/signing",
    [string]$ExpectedCertificate
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$cfg = Get-Content "$repoRoot/target/android-tools/environment.json" | ConvertFrom-Json
$env:JAVA_HOME = $cfg.javaHome
$env:PATH = "$env:JAVA_HOME/bin;$env:PATH"
$signRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SigningRoot)
$UnsignedApk = (Resolve-Path -LiteralPath $UnsignedApk).Path
$OutputApk = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputApk)
if (-not $ExpectedCertificate) {
    $ExpectedCertificate = [IO.File]::ReadAllText((Join-Path $signRoot 'certificate-sha256.txt')).Trim().ToLowerInvariant()
}
if ($ExpectedCertificate -notmatch '^[0-9a-fA-F]{64}$') { throw 'Expected signing certificate must be a SHA256 hex fingerprint' }
$keystore = Join-Path $signRoot 'android-release.p12'
$passwordFile = Join-Path $signRoot 'android-release.dpapi'
if (-not (Test-Path -LiteralPath $passwordFile) -or -not (Test-Path -LiteralPath $keystore)) {
    throw "Existing release signing material missing at $signRoot; restore it to preserve installed app identity"
}
$secure = Get-Content $passwordFile | ConvertTo-SecureString
$credential = [PSCredential]::new('signing', $secure)
$env:ARTTERM_SIGN_PASSWORD = $credential.GetNetworkCredential().Password
try {
    $output = $OutputApk
    New-Item -ItemType Directory -Force (Split-Path $output -Parent) | Out-Null
    $aligned = "$output.aligned"
    & "$($cfg.androidHome)/build-tools/35.0.0/zipalign.exe" -f -P 16 4 $UnsignedApk $aligned
    if ($LASTEXITCODE -ne 0) { throw 'APK alignment failed' }
    & "$($cfg.androidHome)/build-tools/35.0.0/apksigner.bat" sign --ks $keystore --ks-key-alias art-term --ks-pass env:ARTTERM_SIGN_PASSWORD --key-pass env:ARTTERM_SIGN_PASSWORD --out $output $aligned
    if ($LASTEXITCODE -ne 0) { throw 'APK signing failed' }
    $certificateReport = & "$($cfg.androidHome)/build-tools/35.0.0/apksigner.bat" verify --verbose --print-certs $output
    if ($LASTEXITCODE -ne 0) { throw 'APK signature verification failed' }
    $certificateReport
    if (($certificateReport -join "`n") -notmatch "Signer #1 certificate SHA-256 digest: $ExpectedCertificate") {
        throw 'Signing certificate changed; do not deliver this APK as an update'
    }
    & "$($cfg.androidHome)/build-tools/35.0.0/zipalign.exe" -c -P 16 4 $output
    if ($LASTEXITCODE -ne 0) { throw 'Signed APK alignment verification failed' }
    Get-FileHash -LiteralPath $output -Algorithm SHA256
} finally {
    Remove-Item Env:ARTTERM_SIGN_PASSWORD -ErrorAction SilentlyContinue
}
