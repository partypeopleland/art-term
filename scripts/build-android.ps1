param(
    [ValidateSet('aarch64','x86_64')][string]$Target='aarch64',
    [switch]$Debug,
    [string]$ConfigPath,
    [string]$OutputApk,
    [string]$SigningRoot = "$env:LOCALAPPDATA/art-term/signing",
    [string]$ExpectedCertificate
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
if ($ConfigPath) { throw 'APK no longer embeds connections. Add or import connections in the app.' }
if (-not $OutputApk) {
    $version = (Get-Content "$repoRoot/crates/art-term-gui/tauri.android.conf.json" -Raw | ConvertFrom-Json).version
    $OutputApk = Join-Path $repoRoot "dist/android/art-term-$version-$Target.apk"
}
$OutputApk = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputApk)
$SigningRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SigningRoot)
$previousConfig = $env:ART_TERM_CONNECTIONS_FILE
. "$PSScriptRoot/build-paths.ps1"
$previousRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
$env:CARGO_ENCODED_RUSTFLAGS = Get-ArtTermBuildRustFlags -ProjectRoot $repoRoot -AndroidTarget $Target
$env:ART_TERM_CONNECTIONS_FILE = $null
try {
$cfg = Get-Content "$repoRoot/target/android-tools/environment.json" | ConvertFrom-Json
$env:JAVA_HOME=$cfg.javaHome
$env:ANDROID_HOME=$cfg.androidHome
$env:ANDROID_USER_HOME=$cfg.androidUserHome
$env:NDK_HOME=$cfg.ndkHome
$env:PATH="$env:JAVA_HOME/bin;$env:ANDROID_HOME/platform-tools;$env:PATH"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS='-C link-arg=-Wl,-z,max-page-size=16384'
$guiRoot=Join-Path $repoRoot 'crates/art-term-gui'
$cli=Join-Path $repoRoot 'apps/client/node_modules/@tauri-apps/cli/tauri.js'
$profile=if($Debug){'debug'}else{'release'}
$arch=if($Target -eq 'aarch64'){'Arm64'}else{'X86_64'}
$abi=if($Target -eq 'aarch64'){'arm64-v8a'}else{'x86_64'}
$nativeTarget=if($Target -eq 'aarch64'){'aarch64-linux-android'}else{'x86_64-linux-android'}
Push-Location "$repoRoot/apps/client"
try { & npm.cmd run build; if($LASTEXITCODE -ne 0){throw 'Frontend build failed'} } finally { Pop-Location }
Push-Location $guiRoot
try {
    if(-not(Test-Path 'gen/android/app/build.gradle.kts')) {
        & node $cli android init --ci --skip-targets-install
        if($LASTEXITCODE -ne 0){throw 'Android initialization failed'}
    }
    & "$repoRoot/scripts/prepare-android.ps1"
    # Keep the output variant consistent with the selected ABI and fallback task.
    $buildArgs=@('android','build','--target',$Target,'--apk','--split-per-abi','--ci')
    if($Debug){$buildArgs += '--debug'}
    $logPath="$repoRoot/target/android-tools/build-$Target-$profile.txt"
    & node $cli @buildArgs 2>&1 | Tee-Object -FilePath $logPath
    if($LASTEXITCODE -ne 0) {
        $log=Get-Content $logPath -Raw
        if($log -notmatch 'Creation symbolic link is not allowed' -or $log -notmatch 'Finished') {
            throw "Android build failed; see $logPath"
        }
        # Windows may disallow symlinks. Use the compiled library and skip only
        # Gradle's Rust rebuild; no developer-mode or global policy changes.
        $jni="gen/android/app/src/main/jniLibs/$abi"
        New-Item -ItemType Directory -Force $jni | Out-Null
        Copy-Item -LiteralPath "$repoRoot/target/$nativeTarget/$profile/libart_term_mobile.so" -Destination "$jni/libart_term_mobile.so"
        Push-Location gen/android
        try {
            $mode=if($Debug){'Debug'}else{'Release'}
            & ./gradlew.bat "assemble$arch$mode" -x "rustBuild$arch$mode" --console=plain
            if($LASTEXITCODE -ne 0){throw 'Gradle APK assembly failed'}
        } finally {Pop-Location}
    }
    $apks=@(Get-ChildItem "gen/android/app/build/outputs/apk" -Recurse -Filter '*.apk' |
        Where-Object {$_.FullName -match [regex]::Escape($arch.ToLowerInvariant()) -and $_.FullName -match $profile})
    if($apks.Count -ne 1){throw 'Expected exactly one APK for selected ABI and mode'}
    if($Debug){$apks[0].FullName}else{
        & "$repoRoot/scripts/sign-android.ps1" -UnsignedApk $apks[0].FullName -OutputApk $OutputApk -SigningRoot $SigningRoot -ExpectedCertificate $ExpectedCertificate
    }
} finally {Pop-Location}
} finally {
    $env:ART_TERM_CONNECTIONS_FILE = $previousConfig
    $env:CARGO_ENCODED_RUSTFLAGS = $previousRustFlags
}
