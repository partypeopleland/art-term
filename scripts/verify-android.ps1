param(
    [string]$Apk = "$PSScriptRoot/../dist/android/art-term-0.1.19-aarch64.apk",
    [string]$ConfigPath,
    [ValidateSet('aarch64','x86_64')][string]$Target = 'aarch64',
    [string]$ExpectedCertificate,
    [string]$SigningRoot = "$env:LOCALAPPDATA/art-term/signing"
)
$ErrorActionPreference = 'Stop'
if ($ConfigPath) { throw 'APK no longer embeds connections; ConfigPath is unsupported.' }
$repoRoot = Split-Path $PSScriptRoot -Parent
$androidConfig = Get-Content "$repoRoot/crates/art-term-gui/tauri.android.conf.json" -Raw | ConvertFrom-Json
$cfg = Get-Content "$repoRoot/target/android-tools/environment.json" | ConvertFrom-Json
$env:JAVA_HOME = $cfg.javaHome
$env:PATH = "$env:JAVA_HOME/bin;$env:PATH"
$apkPath = (Resolve-Path -LiteralPath $Apk).Path
$buildTools = Join-Path $cfg.androidHome 'build-tools/35.0.0'
$signature = & "$buildTools/apksigner.bat" verify --verbose --print-certs $apkPath
if ($LASTEXITCODE -ne 0) { throw 'Invalid APK signature' }
if (-not $ExpectedCertificate) { $ExpectedCertificate = [IO.File]::ReadAllText((Join-Path $SigningRoot 'certificate-sha256.txt')).Trim().ToLowerInvariant() }
if ($ExpectedCertificate -notmatch '^[0-9a-fA-F]{64}$') { throw 'Expected signing certificate must be a SHA256 hex fingerprint' }
$cert = $ExpectedCertificate
$abi = if ($Target -eq 'aarch64') { 'arm64-v8a' } else { 'x86_64' }
$minimumPageAlignment = if ($Target -eq 'aarch64') { 16384 } else { 4096 }
if (($signature -join "`n") -notmatch "Signer #1 certificate SHA-256 digest: $cert") { throw 'Unexpected signing certificate' }
& "$buildTools/zipalign.exe" -c -P 16 4 $apkPath
if ($LASTEXITCODE -ne 0) { throw 'APK ZIP alignment failed' }
$badging = & "$buildTools/aapt.exe" dump badging $apkPath
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect APK metadata' }
$metadata = $badging -join "`n"
foreach ($expected in @("package: name='net.missart.artterm' versionCode='$($androidConfig.bundle.android.versionCode)' versionName='$($androidConfig.version)'", "sdkVersion:'24'", "targetSdkVersion:'36'", "native-code: '$abi'")) {
    if (-not $metadata.Contains($expected)) { throw "Unexpected APK metadata: $expected" }
}
if ($metadata -match 'application-debuggable') { throw 'Release APK is debuggable' }
$manifest = & "$buildTools/aapt.exe" dump xmltree $apkPath AndroidManifest.xml
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect APK manifest' }
foreach ($flag in @('allowBackup','fullBackupContent','usesCleartextTraffic')) {
    if (($manifest -join "`n") -notmatch "$flag\([^\r\n]+\)=\(type 0x12\)0x0") { throw "Manifest flag must be false: $flag" }
}
$verifyRoot = Join-Path $repoRoot "target/android-tools/verify-$Target-$((Get-FileHash -LiteralPath $apkPath).Hash.Substring(0,12).ToLowerInvariant())"
New-Item -ItemType Directory -Force $verifyRoot | Out-Null
$archive = [IO.Compression.ZipFile]::OpenRead($apkPath)
$nativeEvidence = @()
try {
    $configuration = $archive.GetEntry('assets/connections.toml')
    if ($null -ne $configuration) { throw 'APK contains retired preset connections asset' }
    $libraries = @($archive.Entries | Where-Object { $_.FullName -match '^lib/.+\.so$' })
    if ($libraries.Count -eq 0) { throw 'APK has no native libraries' }
    foreach ($library in $libraries) {
        if ($library.FullName -notmatch "^lib/$abi/[A-Za-z0-9_.-]+\.so`$") { throw 'Unexpected native ABI or path' }
        $localLibrary = Join-Path $verifyRoot $library.Name
        [IO.Compression.ZipFileExtensions]::ExtractToFile($library, $localLibrary, $true)
        if ($library.Name -eq 'libart_term_mobile.so') {
            $nativeText = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($localLibrary))
            foreach ($fixtureSelector in @('ART_TERM_OPENAB_FIXTURE_CA','ART_TERM_OPENAB_FIXTURE_DATA','openab-fixture-ca.der')) {
                if ($nativeText.Contains($fixtureSelector)) { throw 'Release APK contains a debug fixture selector' }
            }
            $nativeText = $null
        }
        $elf = & "$($cfg.ndkHome)/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-readelf.exe" -lW $localLibrary
        if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect native ELF' }
        $loads = @($elf | Where-Object { $_ -match '^\s*LOAD\s' })
        if ($loads.Count -eq 0) { throw 'Native ELF has no LOAD segments' }
        foreach ($load in $loads) {
            $alignment = ($load.Trim() -split '\s+')[-1]
            if ([Convert]::ToInt64($alignment.Substring(2), 16) -lt $minimumPageAlignment) { throw 'Native ELF LOAD alignment is below the selected target requirement' }
        }
        $elf | Set-Content (Join-Path $verifyRoot "$($library.Name).readelf.txt")
        $nativeEvidence += [pscustomobject]@{ file = $library.FullName; sha256 = (Get-FileHash -LiteralPath $localLibrary).Hash.ToLowerInvariant(); loadSegments = $loads.Count; minimumLoadAlignment = $minimumPageAlignment }
    }
} finally { $archive.Dispose() }
$signature | Set-Content (Join-Path $verifyRoot 'signature.txt')
$badging | Set-Content (Join-Path $verifyRoot 'badging.txt')
$manifest | Set-Content (Join-Path $verifyRoot 'manifest.txt')
$result = [pscustomobject]@{
    file = $apkPath
    sha256 = (Get-FileHash -LiteralPath $apkPath).Hash.ToLowerInvariant()
    bytes = (Get-Item -LiteralPath $apkPath).Length
    applicationId = 'net.missart.artterm'
    version = $androidConfig.version
    versionCode = $androidConfig.bundle.android.versionCode
    minSdk = 24
    targetSdk = 36
    certificateSha256 = $cert
    bundledConnectionsAsset = $false
    nativeLibraries = $nativeEvidence
    zipAlignment16KB = $true
    runtimeVerified = $false
}
$result | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $verifyRoot 'verification.json')
$result
