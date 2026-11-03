# Development setup

How to build vpnhide from source.

## Prerequisites

- **JDK 17 or later** — what the CI image installs (`openjdk-17-jdk-headless`); local builds with JDK 21 also work. The `lsposed/app` Gradle build sets `sourceCompatibility = 17` and `jvmTarget = "17"`.
- **Android SDK** — install `platforms;android-37`, compatible build tools (the CI image includes `build-tools;36.0.0`) and `platform-tools`. Export `ANDROID_HOME`. The app currently compiles against API37, targets API36 and supports API28+; see `lsposed/app/build.gradle.kts`.
- **Android NDK r28 or later** — export `ANDROID_NDK_HOME` (or drop it in `$ANDROID_HOME/ndk/<version>/`, the scripts auto-detect). The `lsposed/app` `buildAppNative` task reads `ANDROID_NDK_HOME` (falling back to `ANDROID_NDK_ROOT`, then the SDK-managed `ndk/<version>`) and passes it to cargo-ndk. r27c builds compile, but the resulting cdylibs trigger an Android 16 KiB-page-size compatibility warning at app start on Pixel 8 Pro / future hardware (`сегмент LOAD не выровнен`); the architecture defaults differ (ARMv7 can still use 4 KiB). Core, the APK FFI and Zygisk explicitly pass the shared `build-contract::ANDROID_MAX_PAGE_SIZE` linker contract for both ARM ABIs. Verify actual LOAD alignment and offset/address congruence; compilation alone is insufficient.
- **Rust 1.99.1**, pinned by `rust-toolchain.toml` for every workspace member,
  including Zygisk. The pin installs rustfmt, Clippy and ARM64/ARMv7/x86_64
  Android standard libraries automatically through rustup. CI uses the same
  compiler version. Install the toolchain and cargo-ndk with:
  ```sh
  rustup show
  cargo install cargo-ndk --version 4.1.2 --locked
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

The app-native crates under `crates/` are built via cargo-ndk by the `buildAppNative` Gradle `Exec` task in `lsposed/app/build.gradle.kts` (wired into `preBuild`). It bundles the core-ffi client and shared native checks together in `libvpnhide_core_ffi.so` for both APK ABIs. The APK ships no executable and runs no root process: root probes and effects run in core (`vpnhide-core`), see [core-2.0-ipc-auth](core-ipc-auth.md). The task tracks every app-native path dependency, manifest, lockfile and build script, so changing a shared crate invalidates the APK native build. There is no extra Gradle plugin; no manual `cargo` invocation is needed for the APK build.

## Repository layout

| Path | Component |
|---|---|
| `zygisk/` | Zygisk native module (Rust, inline `libc` hooks) |
| `lsposed/` | LSPosed module + target-picker Android app (Kotlin, Compose) |
| `kmod/` | Kernel-level native backends: GKI `.ko` (C, kretprobes) and KPM (KernelPatch inline hooks) |
| `builtin/` | The in-tree kernel backend (`CONFIG_VPNHIDE=y`): the driver, `scripts/integrate.py` that vendors it into a kernel tree, QEMU tests (no module: core adopts the driver) |
| `crates/` | The Rust workspace: `protocol` (backend wire), `activator` (core-side projection, loaders, Ports), `checks` (shared native probes), `core-rpc` (shared types), `core-client` (transport), `core-ffi` (typed generated UniFFI API), `core-daemon` (privileged engine) |
| `core/` | The required VPN Hide Core Magisk module (`build.py` → `core/target/vpnhide-core.zip`) |
| `scripts/` | Release & changelog tooling |
| `update-json/` | Magisk/KSU update metadata |
| `docs/` | Contributor documentation (this directory) |

Each module has its own README with architecture and design notes.

## Signing keystore (required for lsposed)

`lsposed/app/build.gradle.kts` routes all APK build types (`debug`, `dev`, `rawDebug`, and `release`) through a single signing config that reads `lsposed/keystore.properties`. Without that file, `./gradlew assembleDebug`, `:app:assembleRawDebug`, and `:app:assembleRelease` fail with:

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

### Core daemon base (internal assembly input)

```sh
python3 core/build.py
# → core/target/vpnhide-core.zip (daemon-only base, not the installable bundle)
```

The complete `vpnhide_core` package is required by 2.0. Core owns canonical config,
backend selection/delivery and Ports. This producer builds only the two daemon
ABIs and module scaffold. Assemble the verified payload bundle below before
installation; a successful base build does not close that gate.

### Zygisk payload producer (internal)

```sh
cd zygisk && ./build.py
# → zygisk/target/vpnhide-zygisk.zip
```

The script auto-detects the NDK from `$ANDROID_NDK_HOME` or `~/Android/Sdk/ndk/*`.
This ZIP supplies both ABIs to Core assembly. It is not a separate user install.

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
# → internal vpnhide-kmod-<kmi>.zip producer archives at the repo root
```

The script auto-spawns the `ghcr.io/ylarod/ddk-min:<kmi>-<TAG>` container via Docker or podman (same image CI uses; Docker is preferred when both are installed). For local kernel-source builds via `direnv` and the GKI matrix details, see [kmod/BUILDING.md](../kmod/BUILDING.md).
One variant is useful for kernel engineering tests; complete Core assembly
requires the entire manifest-derived KMI catalog. Users do not choose or flash
one of these archives.

### KPM backend

```sh
./kmod/kpm/build.py
# → vpnhide-kpm.zip at the repo root
```

The script builds `vpnhide.kpm` against the pinned KernelPatch submodule,
stages `kmod/kpm/module/` and packages an internal producer archive. Producer
archives may contain manager installer shims; runtime loading belongs to Core.
KPM requires an actually compatible KernelPatch runtime. Provisioning that engine
is a separate operation; installing Core does not silently patch a boot image.
For runtime requirements and safety notes, see
[kmod/kpm/README.md](../kmod/kpm/README.md).

### Assemble the complete Core package

Build the Core base, all KMI producers, KPM and both Zygisk ABIs from the same
clean commit. Use the shared Rust host tool, as the `core` CI job does:

```sh
mkdir -p .artifacts/core-inputs .artifacts/dist
cp core/target/vpnhide-core.zip .artifacts/core-inputs/vpnhide-core-base.zip
cp vpnhide-kmod-*.zip vpnhide-kpm.zip zygisk/target/vpnhide-zygisk.zip .artifacts/core-inputs/
tools/vpnhide-tooling bundle assemble --inputs .artifacts/core-inputs \
  --output .artifacts/dist/vpnhide-core.zip --commit "$(git rev-parse HEAD)"
```

The tool checks exact source provenance, bytes/hashes, ELF contracts and complete
payload coverage. Dirty or mixed-commit inputs are refused. The output contains
one passive Zygisk ABI pair; backend producer ZIPs are not public installation
choices. Keep these assembly inputs out of the source tree's tracked files.

Package assembly is distinct from authenticated maintenance distribution. CI
signs the exact completed package descriptor with the existing Android key and
embeds the target in the matching APK. Local assembly alone is not a signed
update target. See [Core maintenance](core-maintenance.md) and the
[roadmap](../ROADMAP.md) for unfinished orchestration and acceptance gates.

## Install on device

```sh
# APK
adb install --user 0 -r lsposed/app/build/outputs/apk/release/app-release.apk

# Stage only the verified complete Core ZIP, then install it with the root manager.
adb push .artifacts/dist/vpnhide-core.zip /sdcard/Download/
```

Install the picker APK only for the main Android user (user 0). Plain `adb install`
can install it into secondary profiles too. The main-profile picker already
manages targets in every profile; extra picker copies share the same canonical
configuration and can race each other's saves. Verify per-user installation state
with `adb shell dumpsys package dev.okhsunrog.vpnhide` after installing.

For Core replacement validation, use the manager's normal staged installation
and verify the actual running daemon after a user-chosen reboot. A changed
module.prop alone is not running-generation proof. An APK-only update that stays
compatible and changes no resident hook code does not itself require a device
reboot. Resident LSPosed/Zygisk changes can require a process/lifecycle boundary;
follow the reported state rather than assuming new APK bytes replaced live hooks.
Never reboot automatically.

Bundled alternatives are passive payloads, not conflicting installed backends.
Legacy or independently loaded runtimes still require recovery checks: do not
blindly load a second backend or remove old modules/state before the replacement
is prepared and verified. Preserve canonical config, journal and legacy sources;
follow [migration](migration.md), [maintenance](core-maintenance.md) and actual
reported readiness. The full maintenance executor/UI is still incomplete.

The built-in backend has no module ZIP: integrate the driver into a kernel tree
with `builtin/scripts/integrate.py` and build that kernel (see
[builtin/README.md](../builtin/README.md)). An APK/Core RPC or journal version
change alone does not require rebuilding a compatible builtin kernel. The new
backend lifecycle/selection policy remains under review in
[RFC0005](rfcs/0005-unified-backend-lifecycle-policy.md).

## CI lints (run before pushing)

CI runs the same checks. See [.github/workflows/ci.yml](../.github/workflows/ci.yml) for the authoritative list.

### CI signing and phone-test artifacts

Pushes to `main` and `dev-2.x`, plus manual dispatches, build a signed Debug APK.
`v*` tags build a signed Release APK. Both variants use R8/resource shrinking;
Debug remains debuggable. Signing requires the existing `ANDROID_KEYSTORE_BASE64`,
`ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS` secrets. Missing or empty
secrets, invalid base64, an unreadable keystore or an unusable private-key alias
fail the Android job; CI never creates a replacement signing key. Signing files
are removed from the runner after the job's steps and are not uploaded.

All pull requests, including same-repository PRs and fork PRs, run the applicable
checks without signing secrets. Android lint and JVM unit tests do not require a
keystore. PRs do not build or upload an APK; use a trusted branch push for a
signed phone-test build. The workflow uses `pull_request`, not
`pull_request_target`, and does not grant PR code extra privileges.

For phone tests, download `vpnhide-apk` (`vpnhide.apk`) and `vpnhide-core`
(`vpnhide-core.zip`) from the **same successful run and commit**. Every push to
`dev-2.x` runs both jobs, including Gradle, identity and workflow-only changes.
Artifacts are separate job outputs: the existence of one does not establish that
its partner or the full pipeline passed. CI does not install anything on a phone.
An update over an existing installation still requires the same signing key and
an acceptable version code; a locally generated validation key cannot provide
that compatibility.

### Which CI jobs a pull request runs

The `changes` job at the top of `ci.yml` decides from the diff (`dorny/paths-filter`)
which areas a pull request exercises, and every area job is gated on it:

| Paths | Area jobs |
|---|---|
| `kmod/*`, `kmod/module/**` | `kmod`, `kmod-qemu` |
| `kmod/kpm/**` | `kpm`, `kpm-qemu`, `kpm-qemu-legacy` |
| `builtin/**` | `builtin`, `builtin-integrator`, `builtin-qemu` |
| `zygisk/**` | `zygisk` |
| `lsposed/**` | `android` |
| `core/**` | `core` |
| `shared`: `kmod/shared/**`, `kmod/test/**`, `kmod/generated/**`, `scripts/codegen*.py` | every area, QEMU matrix included |
| `shared_rust`: `crates/**`, `Cargo.*`, `fixtures/core-probes/**`, `scripts/build_lib.py`, `scripts/build-version.py` | every area's packaging job plus `rust`; the QEMU probes and matrix only if a kernel area changed on its own |
| `.github/**`, `scripts/ci_android_keystore.py`, `scripts/test_ci_android_keystore.py` | every area |

`lint-python` (ruff, generated files), `lint-shell-c` (shellcheck, clang-format,
the host-side C tests) and `setup` always run. `rust` (rustfmt, clippy, cargo
tests for the workspace and app-native crates) runs for the shared inputs, `zygisk`
and `lsposed`; `android` (ktlint, detekt, CPD, Android lint and unit tests) for
`lsposed`; `qemu-native-probes` when any kernel-side area does. Pushes to `main`
and `dev-2.x`, `v*` tags and manual dispatches run everything, including both
APK and core packaging. The `ci-ok` job
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
cd .. && cargo fmt --check && cargo ndk -t arm64-v8a clippy -p vpnhide_checks --tests -- -D warnings
cd ../zygisk && cargo test
cargo test -p vpnhide_checks

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

## Isolated host core fixtures

A plain `vpnhide-core [socket]` invocation is not a sandbox: config and journal are
writable even when Native/Ports/root effects are disabled. For a manual host fixture:

```sh
fixture_dir=$(mktemp -d)
env -i PATH="$PATH" VPNHIDE_CONFIG_PATH="$fixture_dir/config.json" \
  VPNHIDE_JOURNAL_PATH="$fixture_dir/operations.json" \
  target/debug/vpnhide-core "$fixture_dir/core.sock"
```

The socket defaults to the current effective UID and `0600`. Tests use
`tests/common::core_command` to clear inherited runtime overrides and then explicitly
add fixture settings. `service`/`service-worker` always select production paths and
are not fixture entry points. Host process tests retain crash/delay hooks; Android
binaries compile those hooks out. These choices do not isolate arbitrary host OS access.
See [IPC authorization](core-ipc-auth.md) for validation and preparation rules.

## Android application identity

All build types, including `dev`, use main's application ID
`dev.okhsunrog.vpnhide` and label **VPN Hide** in every supported locale. The
Kotlin namespace and LSPosed entry class remain unchanged. `dev` remains an
unminified prerelease build, but is not an isolated app or sandbox; it uses the
same signing configuration as debug/release and the normal core endpoint.

An upgrade over a release APK requires the same signing certificate and a
compatible versionCode. A temporary validation keystore only verifies the build;
it does not produce an APK that can update an official installation. Keep APK
and core from the same 2.x build for their RPC contract. Core already binds
`/data/user_de/0/dev.okhsunrog.vpnhide/vpnhide-core.sock`; this identity alignment
does not change its embedded path or protocol.

Previously installed `dev.okhsunrog.vpnhide.dev` / **VPN Hide Dev** is a distinct
Android package and may remain installed separately. Its private preferences,
drafts and files are not automatically transferred. The root canonical file
`/data/system/vpnhide_config.json` is shared state, not that APK's sandbox; existing
settings and isolated 0.x/1.x import behavior are retained. No old package,
sandbox or canonical entries are deleted by this change. The old `.dev` package
still cannot use the production AppOwner socket. Do not enable both copies as
LSPosed modules when validating the new APK.


### Rust 1.99.1 Android Clippy TLS validation

The local NixOS compiler reported `missing_const_for_thread_local` against
explicitly const TLS initializers. The official cloud rustup toolchain
`58cf8f9c35b355199727627d07ae34a4e2846d67` does not reproduce it: the scoped
expectations become unfulfilled. They have been removed after strict ARM64/ARMv7
checks passed without an exemption. Keep the explicit const initializers and
warnings-as-errors; do not suppress unfulfilled expectations across environments.

## Shared host tooling

`tools/vpnhide-tooling` bootstraps the portable Rust CLI through Cargo. Set
`VPNHIDE_TOOLING` to a prebuilt executor to skip Cargo in minimal build images.
`tools/build-host-tooling` builds the static x86_64 Linux executor used by DDK
containers and CI; its bootstrap requires the pinned Rust toolchain and musl
standard-library target. The host CLI does not link the Core daemon.

Component build scripts produce internal inputs and invoke `bundle stamp`.
Assemble a complete installation package with:

```sh
tools/vpnhide-tooling bundle assemble --inputs .artifacts/core-inputs \
  --output dist/vpnhide-core.zip --commit "$(git rev-parse HEAD)"
```

All inputs must be complete, clean and built from that exact commit. A daemon
base alone is not installable. Existing Android keystore operations use
`tools/vpnhide-tooling signer public` / `sign`; the Java provider stays unchanged.

Producer provenance checks the complete Git worktree, including untracked source
inputs and modified/untracked submodule content. Ignored build caches do not make
a clean checkout dirty. A failed, timed-out or truncated Git observation cannot
produce clean identity. Version-label formatting is not source qualification;
assembly independently rejects dirty producer metadata and checks exact bytes.

## Resumable development reset

`scripts/clean-device.sh` now prints an offline plan by default. To request a
reset from the existing Core writer, use an explicit device and private host
receipt; APK uninstall is a separate optional step after confirmed reset:

```sh
scripts/clean-device.sh --execute --serial SERIAL \
  --state-file .artifacts/device-reset/receipt.json --uninstall-app
```

Resume the same command and receipt after interruption. An unknown result queries
the same operation; it never repeats the reset or deletes the journal. The native
client verifies the expected boot and Core server UID. A read-only capability
guard on a pinned executable refuses older CLI binaries before invoking the new
verb. The scoped RPC23 query attests FullReset and its still-empty canonical
commit. A cached confirmation is refreshed before a pending APK uninstall;
completion after APK removal is labelled historical, not a fresh device check.

The receipt directory must be owned and not writable by other users. Core state,
load budgets/holds and installed modules are retained. If Core is absent or
incompatible, keep the journal and install a matching Core/APK through the
manager; this tool does not implement a destructive offline rescue. No device is
contacted without `--execute`. RPC23 does not require a new compatible builtin
kernel.
