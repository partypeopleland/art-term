param([switch]$BootstrapOnly, [string]$SdkRoot)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$toolRoot = Join-Path $repoRoot 'target/android-tools'
$sdkRoot = if ($SdkRoot) { [IO.Path]::GetFullPath($SdkRoot) } else { Join-Path $repoRoot 'target/android-sdk' }
New-Item -ItemType Directory -Force -Path $toolRoot | Out-Null
$jdkArchive = Join-Path $toolRoot 'jdk17.zip'
$jdkRoot = Join-Path $toolRoot 'jdk17'
if (-not (Test-Path $jdkRoot)) {
    $jdkAssets = Invoke-RestMethod 'https://api.adoptium.net/v3/assets/latest/17/hotspot?architecture=x64&image_type=jdk&os=windows&vendor=eclipse'
    $jdkAsset = @($jdkAssets)[0]
    Write-Host 'Downloading isolated JDK 17'
    & curl.exe --fail --location --retry 2 --silent --show-error $jdkAsset.binary.package.link --output $jdkArchive
    if ($LASTEXITCODE -ne 0) { throw 'JDK download failed' }
    if ((Get-FileHash $jdkArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $jdkAsset.binary.package.checksum) { throw 'JDK checksum mismatch' }
    Expand-Archive -LiteralPath $jdkArchive -DestinationPath $jdkRoot
}
$env:JAVA_HOME = (Get-ChildItem $jdkRoot -Directory | Select-Object -First 1).FullName
$env:ANDROID_HOME = $sdkRoot
$env:ANDROID_USER_HOME = Join-Path $toolRoot 'android-user'
New-Item -ItemType Directory -Force -Path $env:ANDROID_USER_HOME | Out-Null
$env:PATH = "$env:JAVA_HOME/bin;$sdkRoot/platform-tools;$env:PATH"
$cmdRoot = Join-Path $toolRoot 'command-line'
if (-not (Test-Path "$cmdRoot/cmdline-tools/bin/sdkmanager.bat")) {
    [xml]$repository = (Invoke-WebRequest 'https://dl.google.com/android/repository/repository2-3.xml').Content
    $package = $repository.SelectNodes("//*[local-name()='remotePackage']") | Where-Object path -eq 'cmdline-tools;latest'
    $archive = $package.archives.archive | Where-Object { $_.'host-os' -eq 'windows' }
    $cmdArchive = Join-Path $toolRoot 'command-line.zip'
    Write-Host 'Downloading Android command line tools'
    & curl.exe --fail --location --retry 2 --silent --show-error "https://dl.google.com/android/repository/$($archive.complete.url)" --output $cmdArchive
    if ($LASTEXITCODE -ne 0) { throw 'Android tool download failed' }
    $algorithm = ([string]$archive.complete.checksum.type).ToUpperInvariant().Replace('-', '')
    if ((Get-FileHash $cmdArchive -Algorithm $algorithm).Hash.ToLowerInvariant() -ne [string]$archive.complete.checksum.InnerText) { throw 'Android tools checksum mismatch' }
    Expand-Archive -LiteralPath $cmdArchive -DestinationPath $cmdRoot
}
$envData = @{javaHome=$env:JAVA_HOME;androidHome=$sdkRoot;androidUserHome=$env:ANDROID_USER_HOME;ndkHome="$sdkRoot/ndk/28.2.13676358";sdkmanager="$cmdRoot/cmdline-tools/bin/sdkmanager.bat"}
$envData | ConvertTo-Json | Set-Content -LiteralPath "$toolRoot/environment.json"
# Legacy avdmanager discovers SDK packages relative to cmdline-tools.
New-Item -ItemType Directory -Force "$sdkRoot/cmdline-tools" | Out-Null
if (-not (Test-Path "$sdkRoot/cmdline-tools/latest")) {
    New-Item -ItemType Junction -Path "$sdkRoot/cmdline-tools/latest" -Target "$cmdRoot/cmdline-tools" | Out-Null
}
if (-not $BootstrapOnly) {
    # New SDK tools use slash-separated sdk-style paths. Semicolons are split
    # by the sdkmanager compatibility wrapper, yielding false package names.
    & "$cmdRoot/cmdline-tools/bin/sdkmanager.bat" "--sdk_root=$sdkRoot" 'platforms/android-35' 'platforms/android-36' 'build-tools/35.0.0' 'ndk/28.2.13676358' 'platform-tools' 'emulator'
    if ($LASTEXITCODE -ne 0) { throw 'SDK install failed; inspect license or download error' }
    foreach ($required in @('platforms/android-35/android.jar', 'platforms/android-36/android.jar', 'build-tools/35.0.0/zipalign.exe', 'ndk/28.2.13676358/source.properties')) {
        if (-not (Test-Path (Join-Path $sdkRoot $required))) { throw "SDK package missing: $required" }
    }
    & rustup target add aarch64-linux-android x86_64-linux-android
    if ($LASTEXITCODE -ne 0) { throw 'Rust Android targets install failed' }
}
Write-Host "Android environment: $toolRoot/environment.json"
