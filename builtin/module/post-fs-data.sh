#!/system/bin/sh
# The in-tree driver is already live if the kernel was built with CONFIG_VPNHIDE
# — there is nothing to insmod, so nothing to do this early. Config delivery
# needs PackageManager (to resolve target UIDs), which is not up yet at
# post-fs-data; doing it here would block boot. It runs from service.sh
# (late-start, backgrounded) instead, exactly like the .ko delivers its config.
exit 0
