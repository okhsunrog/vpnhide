# Development setup

How to build vpnhide from source.

## Prerequisites

- **JDK 17 or later** — what the CI image installs (`openjdk-17-jdk-headless`); local builds with JDK 21 also work. The `lsposed/app` Gradle build sets `sourceCompatibility = 17` and `jvmTarget = "17"`.
- **Android SDK** — install `platforms;android-35`, `build-tools;35.0.0`, `platform-tools` (via Android Studio or `cmdline-tools`). Export `ANDROID_HOME`.
- **Android NDK r28 or later** — export `ANDROID_NDK_HOME` (or drop it in `$ANDROID_HOME/ndk/<version>/`, the scripts auto-detect). The `lsposed/app` `buildAppNative` task reads `ANDROID_NDK_HOME` (falling back to `ANDROID_NDK_ROOT`, then the SDK-managed `ndk/<version>`) and passes it to cargo-ndk. r27c builds compile, but the resulting cdylibs trigger an Android 16 KiB-page-size compatibility warning at app start on Pixel 8 Pro / future hardware (`сегмент LOAD не выровнен`); r28+ aligns LOAD segments on 16 KiB by default. (`crates/checks-jni/build.rs` and `zygisk/build.rs` also pass `-Wl,-z,max-page-size=16384` explicitly so older NDKs stay compatible — defence in depth.)
- **Rust** (latest stable) with the Android target:
  ```sh
  rustup target add aarch64-linux-android
  cargo install cargo-ndk
  ```
- **clang-format 18.x** — kernel-side C formatting (kmod and the built-in
  driver) is pinned because clang-format versions disagree on kernel-style C
  casts. The repo checks this via `scripts/clang-format-c.sh`. On Arch:
  ```sh
  sudo pacman -S clang18
  mkdir -p ~/.local/bin
  ln -sfn /usr/lib/llvm18/bin/clang-format ~/.local/bin/clang-format-18
  clang-format-18 --version
  ```
- **`docker` or `podman`** — only for building the kernel module via DDK images. Docker is preferred when both are installed, matching CI and device-test workflows. See [kmod/BUILDING.md](../kmod/BUILDING.md).
- **`zip`** — packaging module zips.
- **`adb`** — installing builds on a device.

The app-native crates under `crates/` are built via cargo-ndk by the `buildAppNative` Gradle `Exec` task in `lsposed/app/build.gradle.kts` (wired into `preBuild`). It bundles `libvpnhide_checks.so` into the APK's `jniLibs/` plus the root-exec'able `vhhelper` bin as an asset. The helper's `probe` and `mutation` subcommands preserve the diagnostic and [mutation transport](root-mutation-transport.md) boundaries. The task tracks every app-native path dependency, manifest, lockfile and build script, so changing a shared crate invalidates the APK native build. There is no extra Gradle plugin; no manual `cargo` invocation is needed for the APK build.

## Repository layout

| Path | Component |
|---|---|
| `zygisk/` | Zygisk native module (Rust, inline `libc` hooks) |
| `lsposed/` | LSPosed module + target-picker Android app (Kotlin, Compose) |
| `kmod/` | Kernel-level native backends: GKI `.ko` (C, kretprobes) and KPM (KernelPatch inline hooks) |
| `builtin/` | The in-tree kernel backend (`CONFIG_VPNHIDE=y`): kernel patches, `apply.sh`, the KMI-agnostic companion module (activator only), `build.py` |
| `crates/` | The Rust workspace: `protocol` (the wire), `activator` (one bin per backend), `checks` / `checks-jni` (the app's native probes), `app-helper` (`vhhelper`) |
| `portshide/` | Localhost port blocker (shell + iptables) |
| `scripts/` | Release & changelog tooling |
| `update-json/` | Magisk/KSU update metadata |
| `docs/` | Contributor documentation (this directory) |

Each module has its own README with architecture and design notes.

## Signing keystore (required for lsposed)

`lsposed/app/build.gradle.kts` routes all APK build types (`debug`, `rawDebug`, and `release`) through a single signing config that reads `lsposed/keystore.properties`. Without that file, `./gradlew assembleDebug`, `:app:assembleRawDebug`, and `:app:assembleRelease` fail with:

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
cd zygisk && ./build.py
# → zygisk/target/vpnhide-zygisk.zip
```

The script auto-detects the NDK from `$ANDROID_NDK_HOME` or `~/Android/Sdk/ndk/*`.

### lsposed APK

```sh
cd lsposed && ./gradlew :app:assembleDebug
# → lsposed/app/build/outputs/apk/debug/app-debug.apk
```

The default debug APK is still debuggable and uses the same package id/signing
as release, but it is R8/resource-shrunk so LSPosed/Vector has less dex to
prepare after an APK update. This is the recommended local and agent-control
build.

For a distribution build:

```sh
cd lsposed && ./gradlew :app:assembleRelease
# → lsposed/app/build/outputs/apk/release/app-release.apk
```

For the old unminified Studio/debugger path, build `:app:assembleRawDebug`
instead. Use it only when you specifically need to debug code before R8 has
shrunk it.

### kernel module

```sh
./kmod/build.py --kmi android14-6.1   # one variant
./kmod/build.py --all                  # every supported GKI
# → vpnhide-kmod-<kmi>.zip at the repo root
```

The script auto-spawns the `ghcr.io/ylarod/ddk-min:<kmi>-<TAG>` container via Docker or podman (same image CI uses; Docker is preferred when both are installed). For local kernel-source builds via `direnv` and the GKI matrix details, see [kmod/BUILDING.md](../kmod/BUILDING.md).

### KPM backend

```sh
./kmod/kpm/build.py
# → vpnhide-kpm.zip at the repo root
```

The script builds the Android `kpm` activator, builds `vpnhide.kpm` against the
pinned KernelPatch submodule, stages `kmod/kpm/module/`, and packages the
flashable zip. For runtime requirements and safety notes, see
[kmod/kpm/README.md](../kmod/kpm/README.md).

## Install on device

```sh
# APK
adb install --user 0 -r lsposed/app/build/outputs/apk/release/app-release.apk

# Native backend: push exactly one of these, then install via the Magisk,
# KernelSU, KernelSU-Next, or APatch manager path appropriate for the backend.
adb push zygisk/target/vpnhide-zygisk.zip /sdcard/Download/
adb push vpnhide-kmod-<kmi>.zip /sdcard/Download/
# The KPM zip additionally needs a KernelPatch runtime (built into APatch;
# KernelSU-Next/Magisk add it via KPatch-Next) — see kmod/kpm/README.md.
adb push vpnhide-kpm.zip /sdcard/Download/
```

Install the picker APK only for the main Android user (user 0). Plain `adb install`
can install it into secondary profiles too. The main-profile picker already
manages targets in every profile; extra picker copies share the same canonical
configuration and can race each other's saves. Verify per-user installation state
with `adb shell dumpsys package dev.okhsunrog.vpnhide` after installing.

After flashing a native backend, reboot the device. Do not keep multiple native
backends installed unless you are explicitly testing conflict handling; the app
uses priority `kmod > built-in > KPM > Zygisk`, and `.ko` + KPM together is
unsafe. The built-in backend has no module to flash: integrate the driver into
a kernel tree with `builtin/scripts/integrate.py`, build and boot that kernel,
then install the companion zip from `python3 builtin/build.py` (see
[builtin/README.md](../builtin/README.md)).

## CI lints (run before pushing)

CI runs the same checks. See [.github/workflows/ci.yml](../.github/workflows/ci.yml) for the authoritative list.

### Which CI jobs a pull request runs

The `changes` job at the top of `ci.yml` decides from the diff (`dorny/paths-filter`)
which areas a pull request exercises, and every area job is gated on it:

| Paths | Area jobs |
|---|---|
| `kmod/*`, `kmod/module/**` | `kmod-activator`, `kmod`, `kmod-qemu` |
| `kmod/kpm/**` | `kpm`, `kpm-qemu`, `kpm-qemu-legacy` |
| `builtin/**` | `builtin`, `builtin-integrator`, `builtin-qemu` |
| `zygisk/**` | `zygisk` |
| `lsposed/**` | `android` |
| `portshide/**` | `portshide` |
| `shared`: `kmod/shared/**`, `kmod/test/**`, `kmod/generated/**`, `scripts/codegen*.py` | every area, QEMU matrix included |
| `shared_rust`: `crates/**`, `Cargo.*`, `fixtures/app-helper/**`, `scripts/build_lib.py`, `scripts/build-version.py` | every area's packaging job plus `rust`; the QEMU probes and matrix only if a kernel area changed on its own |
| `.github/**` | every area |

`lint-python` (ruff, generated files), `lint-shell-c` (shellcheck, clang-format,
the host-side C tests) and `setup` always run. `rust` (rustfmt, clippy, cargo
tests for the workspace and app-native crates) runs for the shared inputs, `zygisk`
and `lsposed`; `android` (ktlint, detekt, CPD, Android lint, unit tests and the APK
in one Gradle run) for `lsposed`; `qemu-native-probes` when any kernel-side area
does. Pushes to `main`, tags and manual dispatches run everything. The `ci-ok` job
is green when every job succeeded or was skipped by `changes` and red on any
failure, so it is the one status branch protection requires.

`builtin-qemu` builds a kernel per KMI; it compiles through ccache with the
cache restored from the newest run for that KMI (`ccache-builtin-<kmi>-*`), so a
run that changes nothing in the tree mostly links. The job prints `ccache -s`.

Container jobs that run cargo share the composite action
`.github/actions/cargo-cache` (safe.directory plus the registry and target
caches). Actions are pinned to commit SHAs with a version comment; Dependabot
opens a monthly pull request to move the pins.

The `Labels` workflow applies `area:*` labels from the same paths
(`.github/labeler.yml`). Labels only ever add runs: put `area:kmod` on a pull
request to force the kernel-module jobs whatever the diff, or `ci:full` to run
the whole matrix. The decision is taken when the run starts, so re-run the
workflow after adding a label.

```sh
# Generated files — run after editing data/interfaces.toml, data/hooks.toml or
# docs/help/manifest.json; CI fails on any diff (job lint-python)
python3 scripts/codegen-interfaces.py
python3 scripts/codegen-hooks.py
python3 scripts/gen-help-index.py
git diff --quiet  # must be clean

# Python (ruff, config in pyproject.toml). uvx runs without installing anything global.
uvx ruff format --check
uvx ruff check

# Rust
cd zygisk && cargo fmt --check && cargo ndk -t arm64-v8a clippy -- -D warnings
cd .. && cargo fmt --check && cargo ndk -t arm64-v8a clippy -p vpnhide_checks -p vpnhide_checks_jni -p vpnhide_app_helper --tests -- -D warnings
cd ../zygisk && cargo test
cargo test -p vpnhide_checks -p vpnhide_checks_jni -p vpnhide_app_helper

# C (kernel module + built-in driver)
scripts/clang-format-c.sh --check
# Host-side test of the generated VPN-iface matcher used by the kernel module
gcc -O2 -Wall -Werror -o /tmp/test_iface_lists kmod/test_iface_lists.c && /tmp/test_iface_lists

# Kotlin
ktlint "lsposed/app/src/**/*.kt"
cd lsposed && ./gradlew :app:lintDebug :app:testDebugUnitTest
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
