#!/bin/sh
# PID1 bootstrap only. The isolated Rust executor owns setup, probes and verdicts.
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev
exec /vpnhide-tooling qemu guest
