$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$androidRoot = Join-Path $repoRoot 'crates/art-term-gui/gen/android'
if (-not (Test-Path "$androidRoot/app/build.gradle.kts")) { throw 'Run tauri android init first' }
# Connections are created/imported at runtime; remove only this retired generated asset.
New-Item -ItemType Directory -Force "$androidRoot/app/src/main/assets" | Out-Null
$retiredConfiguration = Join-Path $androidRoot 'app/src/main/assets/connections.toml'
if (Test-Path -LiteralPath $retiredConfiguration) { Remove-Item -LiteralPath $retiredConfiguration }
Copy-Item -LiteralPath "$repoRoot/LICENSE" -Destination "$androidRoot/app/src/main/assets/LICENSE"
Copy-Item -LiteralPath "$repoRoot/docs/third-party-notices.md" -Destination "$androidRoot/app/src/main/assets/THIRD-PARTY-NOTICES.md"
# Pin the generated template to available, compatible Android tooling.
$gradle = Get-Content "$androidRoot/build.gradle.kts" -Raw
$gradle = $gradle -replace 'com.android.tools.build:gradle:[^"\s]+', 'com.android.tools.build:gradle:8.11.1'
Set-Content "$androidRoot/build.gradle.kts" $gradle -NoNewline
$buildSrc = Get-Content "$androidRoot/buildSrc/build.gradle.kts" -Raw
$buildSrc = $buildSrc -replace 'com.android.tools.build:gradle:[^"\s]+', 'com.android.tools.build:gradle:8.11.1'
Set-Content "$androidRoot/buildSrc/build.gradle.kts" $buildSrc -NoNewline
$wrapper = Get-Content "$androidRoot/gradle/wrapper/gradle-wrapper.properties" -Raw
$wrapper = $wrapper -replace 'gradle-[\d.]+-bin.zip', 'gradle-8.13-bin.zip'
Set-Content "$androidRoot/gradle/wrapper/gradle-wrapper.properties" $wrapper -NoNewline
$app = Get-Content "$androidRoot/app/build.gradle.kts" -Raw
$app = $app -replace '(compileSdk|targetSdk) = \d+', '$1 = 36'
$app = $app -replace 'versionCode = [^\r\n]+', 'versionCode = 1019'
$app = $app -replace 'versionName = [^\r\n]+', 'versionName = "0.1.19"'
$app = $app -replace 'optimization\s*\{\s*enable = true\s*\}', 'isMinifyEnabled = false'
# The prototype uses Jackson/Tauri reflection. R8 removed enum values() on
# the real signed APK; keep release code intact until keep rules are verified.
$app = $app -replace 'isMinifyEnabled = true', 'isMinifyEnabled = false'
Set-Content "$androidRoot/app/build.gradle.kts" $app -NoNewline
$properties = Get-Content "$androidRoot/gradle.properties" -Raw
if ($properties -notmatch '(?m)^android.useAndroidX=') { $properties += "`nandroid.useAndroidX=true`n" }
# Tauri sources are in Cargo's C: cache while this project is on D:.
# Kotlin incremental path conversion cannot relativize across drive roots.
if ($properties -notmatch '(?m)^kotlin.incremental=') { $properties += "kotlin.incremental=false`n" }
if ($properties -notmatch '(?m)^kotlin.compiler.execution.strategy=') { $properties += "kotlin.compiler.execution.strategy=in-process`n" }
Set-Content "$androidRoot/gradle.properties" $properties -NoNewline
$manifest = Get-Content "$androidRoot/app/src/main/AndroidManifest.xml" -Raw
if ($manifest -notmatch 'android:allowBackup') {
    $manifest = $manifest.Replace('<application', '<application android:allowBackup="false" android:fullBackupContent="false" android:dataExtractionRules="@xml/data_extraction_rules"')
}
Set-Content "$androidRoot/app/src/main/AndroidManifest.xml" $manifest -NoNewline
New-Item -ItemType Directory -Force "$androidRoot/app/src/main/res/xml" | Out-Null
@'
<?xml version="1.0" encoding="utf-8"?>
<data-extraction-rules>
  <cloud-backup><exclude domain="root" path="."/><exclude domain="file" path="."/><exclude domain="database" path="."/><exclude domain="sharedpref" path="."/><exclude domain="external" path="."/></cloud-backup>
  <device-transfer><exclude domain="root" path="."/><exclude domain="file" path="."/><exclude domain="database" path="."/><exclude domain="sharedpref" path="."/><exclude domain="external" path="."/></device-transfer>
</data-extraction-rules>
'@ | Set-Content "$androidRoot/app/src/main/res/xml/data_extraction_rules.xml"
@'
package net.missart.artterm

import android.os.Bundle
import android.view.View
import android.view.WindowManager
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
    WindowInsetsControllerCompat(window, window.decorView).isAppearanceLightStatusBars = false
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard.bottom))
      insets
    }
    ViewCompat.requestApplyInsets(content)
  }

  // Foreground screens stay visible while the app is resumed (novel viewer
  // intent); the flag is cleared on pause — no wake lock, no permissions,
  // no global settings, no lock-screen bypass.
  override fun onResume() {
    super.onResume()
    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
  }

  override fun onPause() {
    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    super.onPause()
  }
}
'@ | Set-Content "$androidRoot/app/src/main/java/net/missart/artterm/mobile/MainActivity.kt"
