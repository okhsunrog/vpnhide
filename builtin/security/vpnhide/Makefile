# SPDX-License-Identifier: MIT
#
# The generated/ and shared/ headers are copied in from kmod/ by
# builtin/scripts/apply.sh at apply time (one source of truth: kmod/generated,
# kmod/shared). -I$(src) lets core.c/hook_*.c include <linux/vpnhide.h> from the
# tree's own include/linux and the vendored generated/shared headers locally.
ccflags-y += -I$(src)

obj-$(CONFIG_VPNHIDE) += vpnhide.o
vpnhide-y := core.o hook_iface.o hook_socket.o hook_route.o
vpnhide-$(CONFIG_VPNHIDE_FS_HIDING) += hook_fs.o
