# Development setup

How to build vpnhide from source.

## Prerequisites

- **JDK 21** — required by the Android Gradle Plugin in `lsposed/`
- **Android SDK** — install `platforms;android-35`, `build-tools;35.0.0`, `platform-tools` (via Android Studio or `cmdline-tools`). Export `ANDROID_HOME`.
- **Android NDK r27c or later** — export `ANDROID_NDK_HOME` (or drop it in `$ANDROID_HOME/ndk/<version>/`, the scripts auto-detect).
- **Rust** (latest stable) with the Android target:
  ```sh
  rustup target add aarch64-linux-android
  cargo install cargo-ndk
  ```
- **`podman` or `docker`** — only for building the kernel module via DDK images. See [kmod/BUILDING.md](../kmod/BUILDING.md).
- **`zip`** — packaging module zips.
- **`adb`** — installing builds on a device.

## Repository layout

| Path | Component |
|---|---|
| `zygisk/` | Zygisk native module (Rust, inline `libc` hooks) |
| `lsposed/` | LSPosed module + target-picker Android app (Kotlin, Compose) |
| `kmod/` | Kernel module (C, kretprobes) |
| `portshide/` | Localhost port blocker (shell + iptables) |
| `scripts/` | Release & changelog tooling |
| `update-json/` | Magisk/KSU update metadata |
| `docs/` | Contributor documentation (this directory) |

Each module has its own README with architecture and design notes.

## Signing keystore (required for lsposed)

`lsposed/app/build.gradle.kts` routes both the `debug` and `release` build types through a single signing config that reads `lsposed/keystore.properties`. Without that file, `./gradlew assembleDebug` and `:app:assembleRelease` fail with:

> SigningConfig 'release' is missing required property 'storeFile'

Create `lsposed/keystore.properties` (git-ignored):

```properties
storeFile=/absolute/path/to/your.jks
keyAlias=yourAlias
password=yourPassword
```

Generate a keystore if you don't have one:

```sh
keytool -genkey -v -keystore ~/vpnhide.jks \
    -keyalg RSA -keysize 4096 -validity 36500 -alias vpnhide
```

## Build each module

### zygisk module

```sh
cd zygisk && ./build-zip.sh
# → zygisk/target/vpnhide-zygisk.zip
```

The script auto-detects the NDK from `$ANDROID_NDK_HOME` or `~/Android/Sdk/ndk/*`.

### lsposed APK

```sh
cd lsposed && ./gradlew :app:assembleRelease
# → lsposed/app/build/outputs/apk/release/app-release.apk
```

### kernel module

Per-GKI-generation builds via DDK Docker/Podman images. See [kmod/BUILDING.md](../kmod/BUILDING.md) for the full guide (GKI identification, DDK commands, local-source builds with `direnv`).

## Install on device

```sh
# APK
adb install -r lsposed/app/build/outputs/apk/release/app-release.apk

# zygisk / kmod: push to device, install via the Magisk or KernelSU manager app
adb push zygisk/target/vpnhide-zygisk.zip /sdcard/Download/
adb push vpnhide-kmod.zip /sdcard/Download/
```

After flashing kmod or zygisk, reboot the device.

## CI lints (run before pushing)

CI runs the same checks:

```sh
# Rust
cd zygisk && cargo fmt --check && cargo ndk -t arm64-v8a clippy -- -D warnings
cd lsposed/native && cargo ndk -t arm64-v8a clippy -- -D warnings

# C
clang-format --dry-run --Werror kmod/vpnhide_kmod.c

# Kotlin
ktlint "lsposed/**/*.kt"
cd lsposed && ./gradlew :app:lint
```

## Build versions

Every module zip and the APK carry a version string derived from git at build time:

- on a release tag `vX.Y.Z` → `X.Y.Z`
- otherwise → `X.Y.Z-N-gSHA` (commits since the nearest tag + short hash, plus `-dirty` if the working tree has uncommitted changes)

So a locally-built dev APK shows up in Android Settings as e.g. `0.6.1-5-gabc1234-dirty`, and the same string lands in `module.prop` inside the zip. The committed `module.prop` files themselves stay at the last release number — the version is stamped into a staging copy per build.

See [releasing.md](releasing.md#build-versions) for details.

## More docs

- [releasing.md](releasing.md) — version bump, tag, release flow
- [changelog.md](changelog.md) — how changelog entries flow from JSON → markdown
- [kmod/BUILDING.md](../kmod/BUILDING.md) — kernel-module build deep dive
