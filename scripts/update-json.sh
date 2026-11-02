#!/usr/bin/env bash
# Generates only the unified Core updateJson for the current VERSION.
# Legacy backend feeds stay frozen; their separate ZIPs are not published in 2.0.
# Run AFTER the GitHub release is published so zipUrl is already valid.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(tr -d '[:space:]' < VERSION)"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: VERSION must be MAJOR.MINOR.PATCH, got '$VERSION'" >&2
    exit 1
fi

IFS='.' read -r MAJOR MINOR PATCH <<< "$VERSION"
# Force base-10: the regex above accepts a zero-padded component (e.g. 1.08.0),
# which bash would otherwise read as octal in $(( )) and abort on a digit 8/9.
VERSION_CODE=$(( 10#$MAJOR * 10000 + 10#$MINOR * 100 + 10#$PATCH ))

REPO="https://github.com/okhsunrog/vpnhide"
RAW="https://raw.githubusercontent.com/okhsunrog/vpnhide/main"

echo "Generating update-json for v${VERSION} (versionCode: $VERSION_CODE)"

mkdir -p update-json
cat > "update-json/update-core.json" <<EOJSON
{
  "version": "v${VERSION}",
  "versionCode": ${VERSION_CODE},
  "zipUrl": "${REPO}/releases/download/v${VERSION}/vpnhide-core.zip",
  "changelog": "${RAW}/update-json/changelog.md"
}
EOJSON
echo "  update-json/update-core.json"
