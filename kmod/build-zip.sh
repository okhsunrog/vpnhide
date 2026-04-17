#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Build the kernel module (env vars loaded by direnv from .env)
if [ ! -f vpnhide_kmod.ko ] || [ vpnhide_kmod.c -nt vpnhide_kmod.ko ]; then
    echo "Building kernel module..."
    make
fi

# Assemble the module staging directory so the committed module.prop
# stays at its release version while the zip carries the actual build
# version (git describe).
STAGING="module-staging"
rm -rf "$STAGING"
cp -a module "$STAGING"
cp vpnhide_kmod.ko "$STAGING/vpnhide_kmod.ko"

BUILD_VERSION="$(../scripts/build-version.sh)"
sed -i "s|^version=.*|version=v${BUILD_VERSION}|" "$STAGING/module.prop"
echo "Stamped module.prop version=v${BUILD_VERSION}"

OUT="vpnhide-kmod.zip"
rm -f "$OUT"
(cd "$STAGING" && zip -qr "../$OUT" .)
rm -rf "$STAGING"

echo
echo "Built: $OUT"
ls -lh "$OUT"
