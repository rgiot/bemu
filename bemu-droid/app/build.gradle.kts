import java.util.zip.GZIPOutputStream

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.bemu.droid"
    compileSdk = 35

    defaultConfig {
        applicationId = "org.bemu.droid"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            // the ABIs rust/build-android.sh builds (BEMU_ABIS to change them)
            abiFilters += abis()
        }
    }

    signingConfigs {
        // a release key from the environment (CI secrets), else the debug key
        val keystore = System.getenv("ANDROID_KEYSTORE_FILE")
        if (keystore != null && file(keystore).exists()) {
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }

    androidResources {
        // the licences' inventory is gzipped already
        noCompress += "gzip"
    }

    packaging {
        jniLibs {
            // the libraries are already stripped (Cargo's release profile)
            keepDebugSymbols += "**/*.so"
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

fun abis(): List<String> =
    (System.getenv("BEMU_ABIS") ?: "arm64-v8a x86_64").split(" ", ",").filter { it.isNotBlank() }

// The emulator, in Rust: libbemu_droid.so for each ABI, in src/main/jniLibs (skipped with
// -PskipCargo when the libraries were built beforehand).
val cargoBuild by tasks.registering(Exec::class) {
    description = "Builds the emulator's native library (rust/build-android.sh)"
    workingDir = rootProject.file("rust")
    commandLine(listOf("./build-android.sh") + abis())
    onlyIf { !project.hasProperty("skipCargo") }
}

// The inventory of the third-party licences (tools/licenses.py), gzipped: a tenth of its 1.6 MB,
// named .gzip (the Android build would unpack a .gz asset into the APK),
// decompressed when the app starts (Licences.kt).
val compressLicences by tasks.registering {
    description = "Gzips third_party/licenses.json"
    val source = rootProject.file("../third_party/licenses.json")
    val target = layout.buildDirectory.file("licences/licenses.json.gzip")
    inputs.file(source)
    outputs.file(target)
    doLast {
        val out = target.get().asFile
        out.parentFile.mkdirs()
        GZIPOutputStream(out.outputStream()).use { it.write(source.readBytes()) }
    }
}

// The licences shown by Settings > Licences, and shipped with the APK: bemu's notice, its licences
// (the app is GPL-3.0: it links the HxC library) and the third-party inventory.
val copyLicences by tasks.registering(Copy::class) {
    description = "Copies NOTICE.md, the licences and the gzipped inventory into the assets"
    from(rootProject.file("..")) {
        include("NOTICE.md", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-GPL-3.0")
    }
    from(compressLicences)
    into(layout.projectDirectory.dir("src/main/assets"))
}

tasks.named("preBuild") {
    dependsOn(cargoBuild, copyLicences)
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.06.00")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")

    testImplementation("junit:junit:4.13.2")
    // the real org.json for the JVM tests (android.jar only has stubs)
    testImplementation("org.json:json:20240303")
}
