#!/usr/bin/env bash
# Offline, isolated signature-enforced kernel experiment. The key/certificate
# belong only to this test kernel, never to production Android/APK signing.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
: "${VPNHIDE_SIGNATURE_IMAGE:?signature-enforcing QEMU Image required}"
: "${VPNHIDE_SIGNATURE_CONFIG:?matching kernel config required}"
: "${VPNHIDE_SIGNATURE_UNSIGNED:?exact unsigned final ko required}"
: "${VPNHIDE_SIGNATURE_SIGNED:?same ko signed by the isolated kernel test key required}"
: "${VPNHIDE_SIGNATURE_ROOTFS:?Alpine aarch64 minirootfs required}"
: "${VPNHIDE_SIGNATURE_CC:?static aarch64 libc compiler required}"
: "${VPNHIDE_SIGNATURE_OUTPUT:?persistent evidence directory required}"
grep -qx 'CONFIG_MODULE_SIG=y' "$VPNHIDE_SIGNATURE_CONFIG"
grep -qx 'CONFIG_MODULE_SIG_FORCE=y' "$VPNHIDE_SIGNATURE_CONFIG"
mkdir -p "$VPNHIDE_SIGNATURE_OUTPUT"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
RFS="$WORK/rootfs"
mkdir "$RFS"
tar xzf "$VPNHIDE_SIGNATURE_ROOTFS" -C "$RFS"
chmod 0755 "$RFS"
cp "$VPNHIDE_SIGNATURE_UNSIGNED" "$RFS/unsigned.ko"
cp "$VPNHIDE_SIGNATURE_SIGNED" "$RFS/signed.ko"
"$REPO/tools/vpnhide-tooling" elf require-contract "$RFS/unsigned.ko" > "$WORK/unsigned-elf.json"
"$REPO/tools/vpnhide-tooling" elf require-contract "$RFS/signed.ko" > "$WORK/signed-elf.json"
python3 - "$RFS" <<'PY'
import sys
from pathlib import Path
root = Path(sys.argv[1])
unsigned = root.joinpath("unsigned.ko").read_bytes()
signed = root.joinpath("signed.ko").read_bytes()
assert signed.startswith(unsigned), "signed input must preserve the exact unsigned ELF"
assert signed.endswith(b"~Module signature appended~\n"), "missing kernel signature trailer"
# Change harmless description text while retaining signature framing and ELF.
bad = bytearray(signed)
at = bad.index(b"description=") + len(b"description=")
bad[at] ^= 1
assert inspect(bytes(bad))["init_contract"] == 1
root.joinpath("tampered.ko").write_bytes(bad)
PY
"$VPNHIDE_SIGNATURE_CC" -static -O2 -Wall -Wextra -Werror -o "$RFS/signature-probe" "$HERE/signature-probe.c"
cat > "$RFS/init" <<'INIT'
#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs dev /dev
mount -t debugfs debugfs /sys/kernel/debug
fail() { echo "RESULT signature_contract=FAIL"; poweroff -f; exit 1; }
sha256sum -c /payloads.sha256 || fail
cat /sys/kernel/debug/kprobes/list > /before || fail
# Each rejection must leave no module, proc nodes or changed probe inventory.
for vector in unsigned tampered preflight; do
    case "$vector" in
        unsigned) path=/unsigned.ko; options=preflight_only=0; expected=129;;
        tampered) path=/tampered.ko; options=preflight_only=0; expected=129;;
        preflight) path=/signed.ko; options=preflight_only=1; expected=117;;
    esac
    /signature-probe "$path" "$options" "$expected" || fail
    [ ! -e /proc/vpnhide_ctl ] && [ ! -e /proc/vpnhide_diag ] &&
        [ ! -e /sys/module/vpnhide_kmod ] &&
        cmp -s /before /sys/kernel/debug/kprobes/list || fail
    echo "RESULT signature_$vector=PASS"
done
# One real load, only after all negative-control postconditions succeeded.
/signature-probe /signed.ko preflight_only=0 0 || fail
[ -e /proc/vpnhide_ctl ] && [ -e /sys/module/vpnhide_kmod ] || fail
cat /proc/vpnhide_ctl
cat /sys/kernel/debug/kprobes/list
# The backend has no module_exit. This VM is discarded after the test.
echo "RESULT signature_contract=PASS"
poweroff -f
INIT
chmod 0755 "$RFS/init"
(cd "$RFS" && sha256sum unsigned.ko signed.ko tampered.ko > payloads.sha256)
cp "$RFS/payloads.sha256" "$VPNHIDE_SIGNATURE_OUTPUT/payloads.sha256"
sha256sum "$VPNHIDE_SIGNATURE_IMAGE" "$VPNHIDE_SIGNATURE_CONFIG" > "$VPNHIDE_SIGNATURE_OUTPUT/kernel.sha256"
(cd "$RFS" && find . | cpio -o -H newc 2>/dev/null | gzip > "$WORK/initramfs.cpio.gz")
LOG="$VPNHIDE_SIGNATURE_OUTPUT/serial.log"
timeout 300 qemu-system-aarch64 -machine virt -cpu max -accel tcg,thread=multi,tb-size=1024 \
    -smp 4 -m 2G -kernel "$VPNHIDE_SIGNATURE_IMAGE" -initrd "$WORK/initramfs.cpio.gz" \
    -append 'console=ttyAMA0 panic=-1 rdinit=/init' -nic none -display none -no-reboot \
    -serial "file:$LOG" > "$VPNHIDE_SIGNATURE_OUTPUT/qemu.log" 2>&1 || true
sed -n '/SIGNATURE_FILE=/p; /RESULT signature_/p' "$LOG"
grep -q '^RESULT signature_contract=PASS' "$LOG"
! grep -Eq 'Kernel panic|BUG:|Oops:' "$LOG"
