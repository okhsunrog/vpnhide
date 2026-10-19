# Boot/kernel partition metadata for kernel bug reports (read-only).
# The by-name listing is for a human reader; names there are fixed partition labels.
# shellcheck disable=SC2010
resolve_partition() {
  NAME="$1"
  SLOT="$2"
  for CANDIDATE in "$NAME$SLOT" "$NAME"; do
    PATH_TO_READ="/dev/block/by-name/$CANDIDATE"
    if [ -e "$PATH_TO_READ" ]; then
      echo "$PATH_TO_READ"
      return 0
    fi
  done
  return 1
}
partition_metadata() {
  NAME="$1"
  echo "### $NAME"
  PATH_TO_READ="$(resolve_partition "$NAME" "$SLOT_SUFFIX" 2>/dev/null || true)"
  if [ -z "$PATH_TO_READ" ]; then
    echo "present=0"
    echo
    return 0
  fi
  REAL_PATH="$(readlink -f "$PATH_TO_READ" 2>/dev/null || echo "$PATH_TO_READ")"
  SIZE="$(blockdev --getsize64 "$PATH_TO_READ" 2>/dev/null || stat -c %s "$PATH_TO_READ" 2>/dev/null || echo unknown)"
  echo "present=1"
  echo "path=$PATH_TO_READ"
  echo "real_path=$REAL_PATH"
  echo "size_bytes=$SIZE"
  ls -lZ "$PATH_TO_READ" 2>&1 || ls -l "$PATH_TO_READ" 2>&1 || true
  if [ "$NAME" = "vendor_dlkm" ]; then
    echo "sha256=skipped (vendor_dlkm can be large; export images separately only when needed)"
  elif [ "$SIZE" != "unknown" ] && [ "$SIZE" -le 268435456 ]; then
    sha256sum "$PATH_TO_READ" 2>&1 || true
  else
    echo "sha256=skipped (size above 268435456 bytes or unknown)"
  fi
  echo
}

SLOT_SUFFIX="$(getprop ro.boot.slot_suffix 2>/dev/null)"
echo "slot_suffix=$SLOT_SUFFIX"
echo "current_slot=$(bootctl get-current-slot 2>/dev/null || true)"
echo "current_slot_suffix=$(bootctl get-suffix "$(bootctl get-current-slot 2>/dev/null)" 2>/dev/null || true)"
for PROP in \
  ro.boot.slot_suffix ro.boot.slot ro.boot.bootdevice ro.boot.super_partition \
  ro.boot.init_boot ro.boot.vendor_boot ro.boot.vbmeta.device_state \
  ro.boot.verifiedbootstate ro.boot.dynamic_partitions
do
  VALUE="$(getprop "$PROP" 2>/dev/null)"
  [ -n "$VALUE" ] && printf '%s=%s\n' "$PROP" "$VALUE"
done
echo
for NAME in boot init_boot vendor_boot vendor_kernel_boot vendor_dlkm; do
  partition_metadata "$NAME"
done
echo "### by-name entries"
ls -lZ /dev/block/by-name 2>&1 | grep -E " boot|init_boot|vendor_boot|vendor_kernel_boot|vendor_dlkm" ||
  ls -l /dev/block/by-name 2>&1 | grep -E " boot|init_boot|vendor_boot|vendor_kernel_boot|vendor_dlkm" || true
