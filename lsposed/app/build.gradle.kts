import java.io.FileInputStream
import java.util.Properties
import org.gradle.api.tasks.Exec
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    // kotlin-android removed: AGP 9+ has built-in Kotlin support.
    alias(libs.plugins.kotlin.atomicfu)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.compose.compiler)
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
    // Generated codegen output (IfaceLists, HookIds) isn't hand-written.
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

// The native check probes ship two ways from one Rust crate (../native):
//   - libvpnhide_checks.so — a cdylib loaded in-process (System.loadLibrary)
//     for the app-view probe (real uid + SELinux domain + zygisk/kernel hooks);
//   - vhprobe — a root-exec'able bin for the ground-truth probe. Shipped as an
//     asset, not a jniLib: AGP 9 defaults to extractNativeLibs=false, so a
//     jniLib isn't a real on-disk file and can't be exec'd.
// Built with cargo-ndk (the same toolchain the zygisk module already uses).
// This replaces the gobley/UniFFI plugin (and its AGP-9 fork): the whole native
// surface is now one JSON-returning function, so codegen bindings aren't worth
// the dependency. -P 29 matches minSdk (getifaddrs needs API >= 24).
val rustJniLibsDir = layout.buildDirectory.dir("rustNative/jniLibs")
val rustAssetsDir = layout.buildDirectory.dir("rustNative/assets")

// Resolve SDK/NDK to plain values at configuration time so the task action
// captures no Project references (configuration-cache safe). NDK version mirrors
// android.ndkVersion below.
val rustNdkVersion = "28.2.13676358"
val rustSdkDir =
    run {
        val lp = rootProject.file("local.properties")
        val fromProps =
            lp.takeIf { it.exists() }?.let { Properties().apply { load(it.inputStream()) }.getProperty("sdk.dir") }
        (fromProps ?: System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT"))?.let(::file)
            ?: error("Android SDK not found: set sdk.dir in local.properties or ANDROID_HOME.")
    }
// Prefer an explicit NDK env (ANDROID_NDK_HOME/_ROOT) the way the zygisk build
// does — the CI image installs the NDK standalone at $ANDROID_NDK_HOME, not under
// $SDK/ndk/<version>. Fall back to the SDK-managed path for local dev.
val rustNdkDir =
    (System.getenv("ANDROID_NDK_HOME") ?: System.getenv("ANDROID_NDK_ROOT"))
        ?.let(::file)
        ?.takeIf { it.isDirectory }
        ?.absolutePath
        ?: rustSdkDir.resolve("ndk/$rustNdkVersion").absolutePath
val nativeCrateDir = projectDir.parentFile.resolve("native")
val rustAssetsOut = rustAssetsDir.get().asFile

// Opt-in x86_64 native + APK ABI for running the app on an Android x86_64
// emulator (arm64 system images don't run on x86 hosts). Enable with
// `-PvpnhideEmulatorX86` (or `vpnhideEmulatorX86=true` in gradle.properties).
// Off by default: release and normal dev builds stay arm64-only. Only the
// `debug` build type honours it. See docs/avd-magisk-testing.md.
val emulatorX86 = (project.findProperty("vpnhideEmulatorX86") as String?)?.toBoolean() == true

val buildRustProbe =
    tasks.register<Exec>("buildRustProbe") {
        group = "build"
        description = "Builds the vpnhide_checks cdylib + vhprobe bin via cargo-ndk."
        workingDir = nativeCrateDir
        environment("ANDROID_NDK_HOME", rustNdkDir)
        environment("NDK_HOME", rustNdkDir)
        inputs.dir(nativeCrateDir.resolve("src")).withPathSensitivity(PathSensitivity.RELATIVE)
        inputs.file(nativeCrateDir.resolve("Cargo.toml"))
        outputs.dir(rustJniLibsDir)
        outputs.dir(rustAssetsDir)
        commandLine(
            buildList {
                add("cargo")
                add("ndk")
                add("-t")
                add("arm64-v8a")
                // Opt-in second ABI for the x86_64 emulator (see emulatorX86).
                if (emulatorX86) {
                    add("-t")
                    add("x86_64")
                }
                // -P 28 matches minSdk; getifaddrs needs API >= 24, so 28 is safe.
                add("-P")
                add("28")
                add("-o")
                add(rustJniLibsDir.get().asFile.absolutePath)
                // --locked: fail if Cargo.lock drifted rather than rewriting it
                // (a dirtied tree stamps the build "-dirty" via git describe).
                add("build")
                add("--release")
                add("--locked")
            },
        )
        // Locals (not top-level script vals) so the doLast action captures only
        // File/Boolean values — required for configuration-cache serialization.
        val probeBinArm = nativeCrateDir.resolve("target/aarch64-linux-android/release/vhprobe")
        val probeDestArm = rustAssetsOut.resolve("bin/arm64-v8a/vhprobe")
        val probeBinX86 = nativeCrateDir.resolve("target/x86_64-linux-android/release/vhprobe")
        val probeDestX86 = rustAssetsOut.resolve("bin/x86_64/vhprobe")
        val copyX86 = emulatorX86
        doLast {
            probeDestArm.parentFile.mkdirs()
            probeBinArm.copyTo(probeDestArm, overwrite = true)
            if (copyX86) {
                probeDestX86.parentFile.mkdirs()
                probeBinX86.copyTo(probeDestX86, overwrite = true)
            }
        }
    }

tasks.named("preBuild").configure { dependsOn(buildRustProbe) }

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
        minSdk = 28
        targetSdk = 36
        versionCode = 10205
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
            isDebuggable = true
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
                "proguard-debug-rules.pro",
            )
            // Opt-in x86_64 (via -PvpnhideEmulatorX86) so the debug APK installs
            // on an x86_64 API-28 emulator. Release stays arm64-only (defaultConfig
            // above) regardless of the flag. See docs/avd-magisk-testing.md.
            if (emulatorX86) {
                ndk {
                    abiFilters += "x86_64"
                }
            }
        }
        create("rawDebug") {
            initWith(getByName("debug"))
            signingConfig = signingConfigs.getByName("release")
            isDebuggable = true
            isMinifyEnabled = false
            isShrinkResources = false
            matchingFallbacks += listOf("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources.excludes += "META-INF/*.kotlin_module"
    }

    // Pick up the cargo-ndk outputs (buildRustProbe): cdylib as a jniLib, the
    // ground-truth probe bin as an asset. buildRustProbe runs via preBuild, so
    // these dirs are populated before the merge/package tasks read them.
    sourceSets["main"].jniLibs.srcDir(rustJniLibsDir.get().asFile)
    sourceSets["main"].assets.srcDir(rustAssetsDir.get().asFile)

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

// Compiler warnings are errors in CI (`-PvpnhideWarningsAsErrors`), not locally:
// a warning mid-edit is information, not a reason to stop the build. The module
// compiles clean, so the gate only has to keep it that way — and the toolchain
// that decides what warns is pinned (Gradle via the wrapper, AGP/Kotlin/Compose
// in libs.versions.toml, NDK by ndkVersion above, JDK by the CI image), so new
// warnings arrive with a deliberate version bump and get dealt with there.
// A warning that is genuinely correct to keep gets an @Suppress at its site.
val warningsAsErrors = (project.findProperty("vpnhideWarningsAsErrors") as String?)?.toBoolean() == true

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
        allWarningsAsErrors.set(warningsAsErrors)
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
    add("rawDebugImplementation", libs.compose.ui.tooling)

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}
