plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "net.missart.artterm.vault"
    compileSdk = 35
    defaultConfig { minSdk = 24 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    testOptions {
        unitTests.all { test ->
            val fixtureRoot = file("../../../target/functional-recovery/android-vault-fixtures")
            test.systemProperty("java.io.tmpdir", fixtureRoot.absolutePath)
            test.doFirst { require(fixtureRoot.mkdirs() || fixtureRoot.isDirectory) }
        }
    }
}

dependencies {
    implementation(project(":tauri-android"))
    testImplementation("junit:junit:4.13.2")
}
