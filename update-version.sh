#!/usr/bin/env bash
# Reads VERSION file and updates all version references in the monorepo.
set -euo pipefail
cd "$(dirname "$0")"

VERSION="$(tr -d '[:space:]' < VERSION)"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: VERSION must be MAJOR.MINOR.PATCH, got '$VERSION'" >&2
    exit 1
fi

IFS='.' read -r MAJOR MINOR PATCH <<< "$VERSION"
VERSION_CODE=$(( MAJOR * 10000 + MINOR * 100 + PATCH ))

echo "Version: $VERSION (versionCode: $VERSION_CODE)"

# kmod module.prop
sed -i "s/^version=.*/version=v${VERSION}/" kmod/module/module.prop
sed -i "s/^versionCode=.*/versionCode=${VERSION_CODE}/" kmod/module/module.prop

# zygisk module.prop
sed -i "s/^version=.*/version=v${VERSION}/" zygisk/module/module.prop
sed -i "s/^versionCode=.*/versionCode=${VERSION_CODE}/" zygisk/module/module.prop

# zygisk Cargo.toml (first version = line only)
sed -i '0,/^version = ".*"/s//version = "'"${VERSION}"'"/' zygisk/Cargo.toml

# lsposed app/build.gradle.kts
sed -i 's/versionCode = [0-9]*/versionCode = '"${VERSION_CODE}"'/' lsposed/app/build.gradle.kts
sed -i 's/versionName = ".*"/versionName = "'"${VERSION}"'"/' lsposed/app/build.gradle.kts

# lsposed native Cargo.toml
sed -i '0,/^version = ".*"/s//version = "'"${VERSION}"'"/' lsposed/native/Cargo.toml

echo "Updated:"
echo "  kmod/module/module.prop"
echo "  zygisk/module/module.prop"
echo "  zygisk/Cargo.toml"
echo "  lsposed/app/build.gradle.kts"
echo "  lsposed/native/Cargo.toml"

# Generate Magisk/KSU updateJson files
REPO="https://github.com/okhsunrog/vpnhide"
RAW="https://raw.githubusercontent.com/okhsunrog/vpnhide/main"

mkdir -p update-json
KMOD_KMIS=("android12-5.10" "android13-5.10" "android13-5.15" "android14-5.15" "android14-6.1" "android15-6.6" "android16-6.12")
for kmi in "${KMOD_KMIS[@]}"; do
    cat > "update-json/update-kmod-${kmi}.json" <<EOJSON
{
  "version": "v${VERSION}",
  "versionCode": ${VERSION_CODE},
  "zipUrl": "${REPO}/releases/download/v${VERSION}/vpnhide-kmod-${kmi}.zip",
  "changelog": "${REPO}/releases/tag/v${VERSION}"
}
EOJSON
    echo "  update-json/update-kmod-${kmi}.json"
done

cat > "update-json/update-zygisk.json" <<EOJSON
{
  "version": "v${VERSION}",
  "versionCode": ${VERSION_CODE},
  "zipUrl": "${REPO}/releases/download/v${VERSION}/vpnhide-zygisk.zip",
  "changelog": "${REPO}/releases/tag/v${VERSION}"
}
EOJSON
echo "  update-json/update-zygisk.json"
