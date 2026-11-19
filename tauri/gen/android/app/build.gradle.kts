import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// The app's version is the workspace's, in the Cargo.toml at the repository
// root; the Tauri CLI does not pass on a version inherited from a workspace.
val cargoVersion: String = Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(rootProject.file("../../../Cargo.toml").readText())?.groupValues?.get(1)
    ?: error("no version in the workspace Cargo.toml")
// 0.3.0 -> 3000, 1.2.3 -> 1002003
val cargoVersionCode: Int = cargoVersion.split('.').map { it.takeWhile(Char::isDigit).toInt() }
    .let { (major, minor, patch) -> major * 1_000_000 + minor * 1_000 + patch }

// Release signing: the key and its password in MDREADER_KEYSTORE_DIR (default
// ~/.config/md-reader-android), as release.jks and keystore-password. Without
// them a release build comes out unsigned.
val keystoreDir = file(System.getenv("MDREADER_KEYSTORE_DIR") ?: "${System.getProperty("user.home")}/.config/md-reader-android")
val releaseKeystore = keystoreDir.resolve("release.jks")

android {
    compileSdk = 36
    namespace = "io.github.mdreader.MdReader"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "io.github.mdreader.MdReader"
        minSdk = 30 // All files access (Android 11)
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode")?.toInt() ?: cargoVersionCode
        versionName = tauriProperties.getProperty("tauri.android.versionName") ?: cargoVersion
    }
    signingConfigs {
        if (releaseKeystore.exists()) {
            create("release") {
                val password = keystoreDir.resolve("keystore-password").readText().trim()
                storeFile = releaseKeystore
                storePassword = password
                keyAlias = "mdreader"
                keyPassword = password
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            signingConfigs.findByName("release")?.let { signingConfig = it }
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")