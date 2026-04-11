#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Build the kernel module (env vars loaded by direnv from .env)
if [ ! -f vpnhide_kmod.ko ] || [ vpnhide_kmod.c -nt vpnhide_kmod.ko ]; then
    echo "Building kernel module..."
    make
fi

cp vpnhide_kmod.ko module/vpnhide_kmod.ko

OUT="vpnhide-kmod.zip"
rm -f "$OUT"
(cd module && zip -qr "../$OUT" .)

echo
echo "Built: $OUT"
ls -lh "$OUT"
