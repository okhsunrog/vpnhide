import java.io.FileInputStream
import java.util.Properties
import org.gradle.api.DefaultTask
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.Exec
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    // kotlin-android removed: AGP 9+ has built-in Kotlin support.
    alias(libs.plugins.kotlin.atomicfu)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.gobley.cargo)
    alias(libs.plugins.gobley.uniffi)
    alias(libs.plugins.detekt)
}

// Static analysis that ktlint (style only) can't do: function/file length,
// cyclomatic/cognitive complexity, dead private members, common bug patterns.
// Runs clean with no baseline — genuinely-inherent length/complexity (shell
// templates, lookup tables, the priority-dispatch classifier) is opted out
// per-site with `@Suppress` + a reason, not hidden in a baseline file.
detekt {
    config.setFrom(rootProject.files("config/detekt/detekt.yml"))
    buildUponDefaultConfig = true
    parallel = true
    source.setFrom(files("src/main/kotlin", "src/test/kotlin"))
}

tasks.withType<io.gitlab.arturbosch.detekt.Detekt>().configureEach {
    // Codegen output (IfaceLists) and UniFFI bindings aren't hand-written.
    exclude("**/generated/**")
    jvmTarget = "17"
    reports {
        html.required.set(true)
        sarif.required.set(false)
        md.required.set(false)
        txt.required.set(false)
    }
}

tasks.withType<io.gitlab.arturbosch.detekt.DetektCreateBaselineTask>().configureEach {
    exclude("**/generated/**")
    jvmTarget = "17"
}

tasks.register<Exec>("ktlintCheck") {
    group = "verification"
    description = "Runs ktlint on app Kotlin sources."
    commandLine("ktlint", "${projectDir}/src/**/*.kt")
}

cargo {
    packageDirectory = layout.projectDirectory.dir("../native")
    // Don't bundle the Rust lib into Android unit-test resources. Unit tests
    // never load the native lib, and bundling drags in a Linux x64 cargo
    // build that fails because the source uses Android-shaped ioctl request
    // types incompatible with glibc.
    builds.withType(gobley.gradle.cargo.dsl.CargoJvmBuild::class.java).configureEach {
        androidUnitTest.set(false)
    }
}

uniffi {
    generateFromLibrary {
        packageName = "dev.okhsunrog.vpnhide.checks"
    }
}

abstract class SuppressGeneratedUniffiWarningsTask : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val bindingFile: RegularFileProperty

    @TaskAction
    fun suppress() {
        // UniFFI emits two intentional object references as bare expressions.
        // Keep the suppression scoped to the generated binding file so real
        // UNUSED_EXPRESSION warnings in hand-written Kotlin still surface.
        val binding = bindingFile.get().asFile
        if (!binding.isFile) return

        val original = binding.readText()
        val updated =
            original.replaceFirst(
                "@file:Suppress(\"RemoveRedundantBackticks\")",
                "@file:Suppress(\"RemoveRedundantBackticks\", \"UNUSED_EXPRESSION\")",
            )
        if (updated != original) {
            binding.writeText(updated)
        }
    }
}

val generatedUniffiBinding =
    layout.buildDirectory.file("generated/uniffi/main/dev/okhsunrog/vpnhide/checks/vpnhide_checks.android.kt")

val suppressGeneratedUniffiWarnings =
    tasks.register<SuppressGeneratedUniffiWarningsTask>("suppressGeneratedUniffiWarnings") {
        dependsOn("buildUniffiBindings")
        bindingFile.set(generatedUniffiBinding)
        outputs.upToDateWhen { false }
    }

tasks.matching { it.name.startsWith("compile") && it.name.endsWith("Kotlin") }.configureEach {
    dependsOn(suppressGeneratedUniffiWarnings)
}

android {
    namespace = "dev.okhsunrog.vpnhide"
    // 37 required transitively by material3 1.5.0-alpha22 (compose 1.12.0-alpha).
    compileSdk = 37

    // Pin the latest stable NDK (r28.2); r28.0 and r29 are still beta. Without
    // this AGP falls back to its bundled default, which lags behind. The Rust
    // cdylibs (zygisk/lsposed native) cross-compile against it via cargo-ndk.
    ndkVersion = "28.2.13676358"

    // Effective build version from ../scripts/build-version.py:
    //   release tag    -> "0.6.2"
    //   dev build      -> "0.6.1-5-gabc1234" (+"-dirty" if uncommitted)
    //   no git         -> VERSION file
    // Python instead of bash so Windows contributors can build without WSL.
    // Script is stdlib-only — no `uv` / pip install needed. `python` on
    // Windows, `python3` elsewhere: Ubuntu 22.04+ ships only the latter,
    // Windows python.org / Store installer ships only the former.
    val isWindows = System.getProperty("os.name").lowercase().contains("windows")
    val pythonExe = if (isWindows) "python" else "python3"
    val buildVersion: String =
        providers
            .exec {
                commandLine(
                    pythonExe,
                    rootProject.projectDir.parentFile.resolve("scripts/build-version.py").absolutePath,
                )
            }.standardOutput.asText
            .get()
            .trim()

    defaultConfig {
        applicationId = "dev.okhsunrog.vpnhide"
        minSdk = 29
        targetSdk = 36
        versionCode = 701
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
            isMinifyEnabled = true
            isShrinkResources = true
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

    packaging {
        resources.excludes += "META-INF/*.kotlin_module"
    }

    // Skip Android Lint on test source sets. Our `src/test/` is pure JVM
    // unit-test logic (filter/recommendation builders) — no Android
    // lifecycle, no layouts, no context misuse. Functional bugs are
    // caught by `:app:testDebugUnitTest`. Saves ~15–20 s of
    // `lintAnalyze*Test` per Lint run.
    //
    // We deliberately leave `checkReleaseBuilds` at its default (true):
    // CI invokes `:app:lintDebug` on PRs (so the release variant isn't
    // analysed there), but ad-hoc `./gradlew :app:lint` on a release
    // build still catches R8/ProGuard-specific issues like MissingRules.
    lint {
        checkTestSources = false
        // Fail the build on unused resources. The detector already runs in the
        // CI `:app:lintDebug` step (as a warning), so gating on it costs no
        // extra time and stops dead strings/resources from accumulating. The
        // app has no dynamic resource lookups (getIdentifier / by-name), so the
        // analysis is exhaustive and false-positive-free here.
        error += "UnusedResources"
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    // Xposed API — compileOnly so it's not bundled into the APK.
    compileOnly("de.robv.android.xposed:api:82")

    // Compose UI
    implementation(libs.core.ktx)
    implementation(libs.activity.compose)
    implementation(platform(libs.compose.bom.get()))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation("androidx.compose.material:material-icons-extended")
    // Reactive theme/settings store (replaces ad-hoc SharedPreferences for UI prefs).
    implementation(libs.datastore.preferences)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.work.runtime.ktx)
    // Material You color-scheme generation + harmonization (seed -> full M3 scheme,
    // AMOLED, contrast, palette styles). Powers VpnHideTheme.
    implementation(libs.material.kolor)
    // iOS-style continuous ("squircle") corners for CornerStyle.Smooth.
    implementation(libs.squircle.shape)
    implementation("io.github.oikvpqya.compose.fastscroller:fastscroller-material3:0.3.2")
    implementation("io.github.oikvpqya.compose.fastscroller:fastscroller-indicator:0.3.2")
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}
