#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Assemble the module staging directory so the committed module.prop
# stays at its release version while the zip carries the actual build
# version (git describe).
STAGING="module-staging"
rm -rf "$STAGING"
cp -a module "$STAGING"

BUILD_VERSION="$(../scripts/build-version.sh)"
sed -i "s|^version=.*|version=v${BUILD_VERSION}|" "$STAGING/module.prop"
echo "Stamped module.prop version=v${BUILD_VERSION}"

OUT="vpnhide-ports.zip"
rm -f "$OUT"
(cd "$STAGING" && zip -qr "../$OUT" .)
rm -rf "$STAGING"

echo
echo "Built: $OUT"
ls -lh "$OUT"
