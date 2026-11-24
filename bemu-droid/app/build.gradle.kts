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

// The licences shown by Settings > Licences, and shipped with the APK: bemu's notice, its licences
// (the app is GPL-3.0: it links the HxC library) and the third-party inventory (tools/licenses.py).
val copyLicences by tasks.registering(Copy::class) {
    description = "Copies NOTICE.md, the licences and third_party/licenses.json into the assets"
    val repository = rootProject.file("..")
    from(repository) {
        include("NOTICE.md", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-GPL-3.0")
    }
    from(File(repository, "third_party")) {
        include("licenses.json")
    }
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
}
