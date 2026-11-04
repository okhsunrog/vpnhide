#!/usr/bin/env bash
# Static musl kernel oracle, not an APK/Android/Zygisk acceptance worker.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to build the static oracle}"
output="${1:-$ROOT/.artifacts/qemu-native-probes/native-fixture-oracle}"
linker="$ROOT/.artifacts/build-tools/aarch64-musl-clang"
mkdir -p "$(dirname "$linker")"
cat > "$linker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
# Rust requests both -static and -no-pie. Clang's static driver already fixes
# ET_EXEC and warns that -no-pie has no effect; remove only that redundant flag.
static=false
for argument in "$@"; do [ "$argument" != -static ] || static=true; done
arguments=()
for argument in "$@"; do
    if [ "$static" = true ] && [ "$argument" = -no-pie ]; then continue; fi
    arguments+=("$argument")
done
exec "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/clang" --target=aarch64-linux-musl -fuse-ld=lld "${arguments[@]}"
SH
chmod 0700 "$linker"
rustup target add aarch64-unknown-linux-musl
# C build scripts (ring) otherwise guess aarch64-linux-musl-gcc, absent on
# hosts and CI. Compile with the same NDK clang target and LLVM archiver.
llvm_bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
CC_aarch64_unknown_linux_musl="$llvm_bin/clang --target=aarch64-linux-musl" \
    AR_aarch64_unknown_linux_musl="$llvm_bin/llvm-ar" \
    CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$linker" \
    cargo build --target aarch64-unknown-linux-musl -p vpnhide_core_daemon --features fixture-oracle --bin vpnhide-fixture-oracle
mkdir -p "$(dirname "$output")"
install -m 0755 target/aarch64-unknown-linux-musl/debug/vpnhide-fixture-oracle "$output"
"$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip" "$output"
