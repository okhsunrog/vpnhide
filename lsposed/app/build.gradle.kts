import java.io.FileInputStream
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.compose.compiler)
}

android {
    namespace = "dev.okhsunrog.vpnhide"
    compileSdk = 35

    // Effective build version from ../scripts/build-version.sh:
    //   release tag    -> "0.6.2"
    //   dev build      -> "0.6.1-5-gabc1234" (+"-dirty" if uncommitted)
    //   no git         -> VERSION file
    val buildVersion: String =
        providers
            .exec {
                commandLine(
                    "bash",
                    rootProject.projectDir.parentFile.resolve("scripts/build-version.sh").absolutePath,
                )
            }.standardOutput.asText
            .get()
            .trim()

    defaultConfig {
        applicationId = "dev.okhsunrog.vpnhide"
        minSdk = 29
        targetSdk = 35
        versionCode = 601
        versionName = buildVersion

        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    signingConfigs {
        create("release") {
            val keystorePropertiesFile = rootProject.file("keystore.properties")
            if (keystorePropertiesFile.exists()) {
                val keystoreProperties = Properties()
                keystoreProperties.load(FileInputStream(keystorePropertiesFile))
                keyAlias = keystoreProperties["keyAlias"] as String
                keyPassword = keystoreProperties["password"] as String
                storeFile = file(keystoreProperties["storeFile"] as String)
                storePassword = keystoreProperties["password"] as String
            }
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
        debug {
            signingConfig = signingConfigs.getByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    packaging {
        resources.excludes += "META-INF/*.kotlin_module"
    }
}

// Build the Rust native checks library via cargo-ndk.
val buildRustNative by tasks.registering {
    outputs.upToDateWhen { false }

    doLast {
        exec {
            workingDir = file("../native")
            commandLine("cargo", "ndk", "-t", "arm64-v8a", "build", "--release")
        }
        val src = file("../native/target/aarch64-linux-android/release/libvpnhide_checks.so")
        val dst = file("src/main/jniLibs/arm64-v8a/libvpnhide_checks.so")
        dst.parentFile.mkdirs()
        src.copyTo(dst, overwrite = true)
    }
}

tasks.named("preBuild") {
    dependsOn(buildRustNative)
}

dependencies {
    // Xposed API — compileOnly so it's not bundled into the APK.
    compileOnly("de.robv.android.xposed:api:82")

    // XML theme for splash screen (DayNight support)
    implementation("com.google.android.material:material:1.12.0")

    // Compose UI
    implementation(libs.core.ktx)
    implementation(libs.activity.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation("androidx.compose.material:material-icons-extended")
    implementation("io.github.oikvpqya.compose.fastscroller:fastscroller-material3:0.3.2")
    implementation("io.github.oikvpqya.compose.fastscroller:fastscroller-indicator:0.3.2")
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)

    testImplementation("junit:junit:4.13.2")
}
