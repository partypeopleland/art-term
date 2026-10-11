#requires -Version 5.1
param(
    [switch]$Prepare,
    [switch]$BuildOnly,
    [switch]$SkipBuild,
    [string]$PythonPath,
    [string]$WindowsExe,
    [string]$EdgeDriverPath
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
if ($env:OS -ne 'Windows_NT') { throw 'Run Android regression on Windows.' }
if ($BuildOnly -and $SkipBuild) { throw 'BuildOnly and SkipBuild cannot be combined.' }
$outputRoot = Join-Path $repoRoot 'target/android-regression'
foreach ($path in @((Join-Path $repoRoot 'target'), $outputRoot)) {
    if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Test target cannot be a reparse point.' }
}
New-Item -ItemType Directory -Force $outputRoot | Out-Null
$runRoot = Join-Path $outputRoot ('checks-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $runRoot | Out-Null
$steps = @()
$passed = $false
function Invoke-Check {
    param([string]$Name, [string]$Program, [string[]]$Arguments)
    $log = Join-Path $runRoot ($Name + '.log')
    Write-Host "[$Name] Running"
    $savedPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $Program @Arguments *> $log
        $exitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = $savedPreference }
    $script:steps += @{ name = $Name; exitCode = $exitCode; log = $log }
    if ($exitCode -ne 0) { throw "$Name failed; inspect $log" }
    Write-Host "[$Name] PASS"
}
function Source-Hashes {
    $files = & git -C $repoRoot ls-files -- apps/desktop crates Cargo.toml Cargo.lock scripts/prepare-android.ps1 scripts/build-paths.ps1
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify APK source inputs.' }
    $hashes = @{}
    foreach ($file in $files) { $hashes[$file] = (Get-FileHash -LiteralPath (Join-Path $repoRoot $file) -Algorithm SHA256).Hash.ToLowerInvariant() }
    return $hashes
}
$previousEnvironment = @{}
foreach ($name in @('JAVA_HOME','ANDROID_HOME','ANDROID_USER_HOME','NDK_HOME','PATH','ART_TERM_CONNECTIONS_FILE','CARGO_ENCODED_RUSTFLAGS')) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
Push-Location $repoRoot
try {
    $cfg = Get-Content -LiteralPath (Join-Path $repoRoot 'target/android-tools/environment.json') -Raw | ConvertFrom-Json
    $env:JAVA_HOME = $cfg.javaHome
    $env:ANDROID_HOME = $cfg.androidHome
    $env:ANDROID_USER_HOME = $cfg.androidUserHome
    $env:NDK_HOME = $cfg.ndkHome
    $env:PATH = "$env:JAVA_HOME/bin;$env:ANDROID_HOME/platform-tools;$env:PATH"
    $env:ART_TERM_CONNECTIONS_FILE = $null
    if (-not $PythonPath) { $PythonPath = Join-Path $repoRoot 'target/windows-regression/venv/Scripts/python.exe' }
    if (-not $WindowsExe) { $WindowsExe = Join-Path $repoRoot 'target/debug/art-term-gui.exe' }
    if (-not $EdgeDriverPath) { $EdgeDriverPath = Join-Path $repoRoot 'target/gui-tools/edge/msedgedriver.exe' }
    if ($Prepare) {
        if (-not (Test-Path -LiteralPath $PythonPath)) {
            $python = @(Get-Command python -CommandType Application)[0].Source
            Invoke-Check 'python-venv' $python @('-m','venv',(Join-Path $repoRoot 'target/windows-regression/venv'))
        }
        Invoke-Check 'python-dependencies' $PythonPath @('-m','pip','install','-r','scripts/android-regression/requirements.txt')
    }
    $apkRoot = Join-Path $outputRoot 'apk'
    if ((Test-Path -LiteralPath $apkRoot) -and ((Get-Item -LiteralPath $apkRoot).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'APK directory cannot be a reparse point.' }
    New-Item -ItemType Directory -Force $apkRoot | Out-Null
    $apk = Join-Path $apkRoot 'art-term-x86_64-fixture.apk'
    $manifestPath = Join-Path $apkRoot 'build-manifest.json'
    if (-not $SkipBuild) {
        . "$PSScriptRoot/build-paths.ps1"
        $env:CARGO_ENCODED_RUSTFLAGS = Get-ArtTermBuildRustFlags -ProjectRoot $repoRoot -AndroidTarget 'x86_64'
        $npm = @(Get-Command npm.cmd -CommandType Application)[0].Source
        $node = @(Get-Command node -CommandType Application)[0].Source
        Invoke-Check 'frontend-build' $npm @('--prefix','apps/desktop','run','build')
        & "$PSScriptRoot/prepare-android.ps1"
        $sourceHashes = Source-Hashes
        Push-Location (Join-Path $repoRoot 'crates/art-term-gui')
        try {
            $cli = Join-Path $repoRoot 'apps/desktop/node_modules/@tauri-apps/cli/tauri.js'
            try {
                Invoke-Check 'fixture-apk-build' $node @($cli,'android','build','--target','x86_64','--apk','--ci','--debug','--features','openab-fixture')
            } catch {
                $buildLog = Get-Content (Join-Path $runRoot 'fixture-apk-build.log') -Raw
                if ($buildLog -notmatch 'Creation symbolic link is not allowed' -or $buildLog -notmatch 'Finished') { throw }
                $library = Join-Path $repoRoot 'target/x86_64-linux-android/debug/libart_term_mobile.so'
                $jni = Join-Path $repoRoot 'crates/art-term-gui/gen/android/app/src/main/jniLibs/x86_64'
                New-Item -ItemType Directory -Force $jni | Out-Null
                Copy-Item -LiteralPath $library -Destination (Join-Path $jni 'libart_term_mobile.so')
                Push-Location 'gen/android'
                try { Invoke-Check 'fixture-apk-assemble' './gradlew.bat' @('assembleX86_64Debug','-x','rustBuildX86_64Debug','--console=plain') } finally { Pop-Location }
                # Preserve the real failed step; the explicit supported packaging
                # fallback is recorded separately rather than claiming it exited 0.
                $steps[-2]['recoveredBy'] = 'fixture-apk-assemble'
            }
        } finally { Pop-Location }
        $candidates = @(Get-ChildItem (Join-Path $repoRoot 'crates/art-term-gui/gen/android/app/build/outputs/apk') -Recurse -Filter '*.apk' | Where-Object { $_.FullName -match 'x86_64' -and $_.FullName -match 'debug' })
        if ($candidates.Count -ne 1) { throw 'Expected exactly one x86_64 debug APK.' }
        Copy-Item -LiteralPath $candidates[0].FullName -Destination $apk
        $aapt = Join-Path $cfg.androidHome 'build-tools/35.0.0/aapt.exe'
        Invoke-Check 'apk-metadata' $aapt @('dump','badging',$apk)
        $metadata = Get-Content (Join-Path $runRoot 'apk-metadata.log') -Raw
        if ($metadata -notmatch "package: name='net.missart.artterm'" -or $metadata -notmatch 'application-debuggable' -or $metadata -notmatch "native-code: 'x86_64'") { throw 'Unexpected test APK package/debug/ABI.' }
        @{ apk = $apk; sha256 = (Get-FileHash $apk -Algorithm SHA256).Hash.ToLowerInvariant(); sourceHashes = $sourceHashes; debugFixture = $true; createdAt = [DateTime]::UtcNow.ToString('o') } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    } else {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if ((Get-FileHash $apk -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.sha256) { throw 'APK hash differs from build manifest.' }
        $current = Source-Hashes
        if ($current.Count -ne @($manifest.sourceHashes.PSObject.Properties).Count) { throw 'APK source input list changed; rebuild.' }
        foreach ($property in $manifest.sourceHashes.PSObject.Properties) {
            if ($current[$property.Name] -ne $property.Value) { throw "APK source changed; rebuild: $($property.Name)" }
        }
    }
    if (-not $BuildOnly) {
        Invoke-Check 'python-preflight' $PythonPath @('-c','import websocket, selenium, aiohttp, cryptography, PIL')
        Invoke-Check 'android-native-e2e' $PythonPath @('scripts/android-regression/environment.py','run','--apk',$apk,'--windows-exe',$WindowsExe,'--edge-driver',$EdgeDriverPath)
    }
    $passed = $true
} catch {
    Write-Host ('FAIL: ' + $_.Exception.Message)
} finally {
    Pop-Location
    foreach ($name in $previousEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process') }
    @{ passed = $passed; buildOnly = [bool]$BuildOnly; steps = $steps; artifactDirectory = $runRoot } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'result.json') -Encoding UTF8
    Write-Host "Result: $runRoot/result.json"
}
if (-not $passed) { exit 1 }
