/* SPDX-License-Identifier: MIT */
/*
 * vpnhide (in-tree) internal API — the brain shared between core.c (config,
 * stats, /proc/vpnhide_ctl control wire, init) and the per-subsystem hook
 * bodies in hook_socket.c / hook_netlink.c / hook_fs.c.
 *
 * NOT a UAPI header and NOT the public call-site API (that is
 * include/linux/vpnhide.h). Nothing outside security/vpnhide/ includes this.
 */
#ifndef _SECURITY_VPNHIDE_INTERNAL_H
#define _SECURITY_VPNHIDE_INTERNAL_H

#include <linux/types.h>

#include "generated/hook_ids.h"

/* Mirror of vpnhide_protocol::MAX_TARGET_UIDS (crates/protocol/src/lib.rs) and
 * the .ko's MAX_TARGET_UIDS; the activator truncates the projected config to
 * this many targets, so keep all three in sync. */
#define MAX_TARGET_UIDS 160

#define MODNAME "vpnhide"

/*
 * Hooks this backend owns and may act on. Same kernel dump/ioctl/route/bind
 * hooks as the .ko (VPNHIDE_KERNEL_HOOK_MASK); the filesystem-concealment bit
 * (VPNHIDE_KPATCH_HOOK_MASK) is added only when the VFS hooks are compiled in.
 */
#ifdef CONFIG_VPNHIDE_FS_HIDING
#define VPNHIDE_OWNED_HOOK_MASK \
	(VPNHIDE_KERNEL_HOOK_MASK | VPNHIDE_KPATCH_HOOK_MASK)
#else
#define VPNHIDE_OWNED_HOOK_MASK (VPNHIDE_KERNEL_HOOK_MASK)
#endif

/* Debug logging, gated by the `debug` line of the last /proc/vpnhide_ctl config
 * snapshot. READ_ONCE keeps the compiler from tearing/hoisting the flag across
 * hot paths — same contract as the .ko. */
extern bool vpnhide_debug_enabled;
#define vpnhide_dbg(fmt, ...)                                          \
	do {                                                          \
		if (READ_ONCE(vpnhide_debug_enabled))                 \
			pr_info(MODNAME ": " fmt, ##__VA_ARGS__);     \
	} while (0)

/*
 * True if `hook_id` is enabled for the calling UID (per-hook gate, protocol
 * §4.3). Fast path: a lock-free reject when no target enables this hook.
 * Defined in core.c; called from every hook body.
 */
bool vpnhide_hook_active(enum vpnhide_hook_id hook_id);

/*
 * Record one interception for the calling UID against `hook_id` (protocol §4.3
 * `stats`). Defined in core.c.
 */
void vpnhide_record_hook_hit(enum vpnhide_hook_id hook_id);

#endif /* _SECURITY_VPNHIDE_INTERNAL_H */
