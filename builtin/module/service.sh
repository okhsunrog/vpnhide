#!/system/bin/sh
# Late (post-boot) retry of config delivery, in case /proc/vpnhide_ctl or the
# config store was not ready yet in post-fs-data. Idempotent: boot-service just
# re-verifies the backend id, re-delivers the config, and refreshes load_status.
MODDIR="${0%/*}"
"$MODDIR/activator" boot-service &
exit 0
