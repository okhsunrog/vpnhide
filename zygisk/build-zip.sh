#!/usr/bin/env bash
# Build the native library for aarch64 Android and package it into an
# installable KernelSU/Magisk module zip.
#
# Requirements:
#   - rustup target aarch64-linux-android (already installed)
#   - cargo-ndk
#   - Android NDK at $ANDROID_NDK_HOME or auto-detected from $HOME/Android/Sdk/ndk/*
#   - zip

set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$PROJECT_DIR"

# Auto-detect NDK if ANDROID_NDK_HOME isn't set
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    ANDROID_NDK_HOME="$(find "$HOME/Android/Sdk/ndk" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | sort -V | tail -1)"
fi
if [ -z "${ANDROID_NDK_HOME:-}" ] || [ ! -d "$ANDROID_NDK_HOME" ]; then
    echo "error: ANDROID_NDK_HOME not set and no NDK found under ~/Android/Sdk/ndk" >&2
    exit 1
fi
echo "Using NDK: $ANDROID_NDK_HOME"
export ANDROID_NDK_HOME

# Build the cdylib for arm64-v8a
cargo ndk -t arm64-v8a build --release

SO_SRC="target/aarch64-linux-android/release/libvpnhide_zygisk.so"
if [ ! -f "$SO_SRC" ]; then
    echo "error: expected $SO_SRC after cargo ndk build, not found" >&2
    exit 1
fi

# Assemble the module staging directory
STAGING="target/module-staging"
rm -rf "$STAGING"
cp -a module "$STAGING"
mkdir -p "$STAGING/zygisk"
cp "$SO_SRC" "$STAGING/zygisk/arm64-v8a.so"

# Stamp the effective build version into the staging module.prop without
# touching the committed file. On a release tag this matches VERSION; on
# any other commit the git suffix makes dev builds identifiable.
BUILD_VERSION="$(../scripts/build-version.sh)"
sed -i "s|^version=.*|version=v${BUILD_VERSION}|" "$STAGING/module.prop"
echo "Stamped module.prop version=v${BUILD_VERSION}"

# CI sets UPDATE_JSON_URL so Magisk/KSU knows where to check for updates;
# local dev builds leave it unset and ship without updateJson.
if [ -n "${UPDATE_JSON_URL:-}" ]; then
    echo "updateJson=${UPDATE_JSON_URL}" >> "$STAGING/module.prop"
fi

# Zip it
OUT_ZIP="target/vpnhide-zygisk.zip"
rm -f "$OUT_ZIP"
(cd "$STAGING" && zip -qr "../../$OUT_ZIP" .)

echo
echo "Built: $OUT_ZIP"
ls -lh "$OUT_ZIP"
