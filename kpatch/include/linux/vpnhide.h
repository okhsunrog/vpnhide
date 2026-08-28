/* SPDX-License-Identifier: MIT */
/*
 * vpnhide — public in-tree API included by patched kernel subsystems.
 *
 * This is the compile-time (kpatch) sibling of the out-of-tree kmod. The .ko
 * attaches the same filtering logic with kretprobes; here the logic is compiled
 * into vmlinux and reached through direct call-site hooks placed by the
 * per-version patches under kpatch/versions/<kmi>/. No kprobes, no module
 * loading — works on kernels where the .ko cannot run at all (INTEGRATE_MODULES,
 * CONFIG_KPROBES=n, whole-program LTO that mangles hook-target symbols).
 *
 * The filtering decision, config, stats and the /proc/vpnhide_ctl control wire
 * live in security/vpnhide/ and are shared VERBATIM with the .ko via
 * shared/vpnhide_logic.h + generated/{iface_lists,hook_ids}.h. Only the ATTACH
 * mechanism differs between the two backends.
 *
 * When CONFIG_VPNHIDE=n every entry point below compiles to an immediate
 * false/0/void so the patched call sites optimise away to nothing.
 */
#ifndef _LINUX_VPNHIDE_H
#define _LINUX_VPNHIDE_H

#include <linux/types.h>
#include <linux/if.h>		/* IFNAMSIZ */
#include <linux/sockptr.h>	/* sockptr_t */

struct net_device;
struct sock;
struct dentry;

/*
 * Global hook ids (data/hooks.toml -> generated/hook_ids.h). The patch at each
 * kernel call site passes the id of the hook it stands in for, so the same few
 * predicates below preserve the .ko's per-hook enable masks AND per-hook stats.
 * Kept as a plain int in the public API to avoid pulling the generated enum
 * into every patched translation unit; security/vpnhide/ uses the real enum.
 */

/*
 * SO_BINDTODEVICE / SO_BINDTOIFINDEX decision. The .ko must redirect the syscall
 * with a kprobe and freeze the option to beat a userspace TOCTOU; in-tree the
 * patch sits at the __sys_setsockopt call site (process context, before the
 * option is applied), so the decision reduces to this one call plus, for the
 * FROZEN verdict, swapping in the kernel-side snapshot the driver captured.
 */
enum vpnhide_bind_action {
	VPNHIDE_BIND_PASSTHROUGH,	/* not a bind opt / not hidden: proceed as-is */
	VPNHIDE_BIND_FROZEN,		/* proceed, but with *snap (TOCTOU-safe copy) */
	VPNHIDE_BIND_DENY,		/* hidden VPN interface: return -ENODEV */
	VPNHIDE_BIND_FAULT,		/* optval copy faulted: return -EFAULT */
};

union vpnhide_bind_snapshot {
	char name[IFNAMSIZ];
	int ifindex;
};

#ifdef CONFIG_VPNHIDE

/*
 * True if `ifname` names a VPN interface that must be hidden from the calling
 * UID for `hook_id`. Used by the per-interface ioctl sites (dev_ifsioc /
 * dev_ifname) where only the requested name is in hand.
 */
bool vpnhide_should_hide_ifname(const char *ifname, int hook_id);

/*
 * True if `dev` is a VPN interface that must be hidden from the calling UID for
 * `hook_id`. The workhorse: interface/address/route dump sites (rtnl_fill_ifinfo,
 * inet{,6}_fill_ifaddr, dev_ifconf, the fib route/rule fill paths) pass the
 * netdev they are about to emit; a true return means "skip this entry".
 */
bool vpnhide_should_hide_dev(const struct net_device *dev, int hook_id);

/*
 * Classify a setsockopt(SO_BINDTODEVICE / SO_BINDTOIFINDEX). On FROZEN, *snap
 * holds a kernel copy the caller must pass to the option handler instead of the
 * user pointer. `sk` may be NULL (treated as passthrough).
 */
enum vpnhide_bind_action vpnhide_setsockopt_bind(struct sock *sk, int optname,
						 sockptr_t optval,
						 unsigned int optlen,
						 union vpnhide_bind_snapshot *snap);

#ifdef CONFIG_VPNHIDE_FS_HIDING
/*
 * True if `dentry` resolves to a VPN interface's sysfs / proc-sys node that must
 * be concealed from the calling UID (lookup / open / getattr sites). Dentry-based
 * so it also covers relative openat, symlink following, and bind mounts.
 */
bool vpnhide_should_hide_dentry(const struct dentry *dentry, int hook_id);

/*
 * True if `dentry` is a directory whose listing enumerates interface nodes
 * (sysfs .../net, proc .../{conf,neigh}); the iterate_dir site uses it to know a
 * listing needs per-entry filtering.
 */
bool vpnhide_is_iface_listing_dir(const struct dentry *dentry);
#endif /* CONFIG_VPNHIDE_FS_HIDING */

#else /* !CONFIG_VPNHIDE */

static inline bool vpnhide_should_hide_ifname(const char *ifname, int hook_id)
{
	return false;
}
static inline bool vpnhide_should_hide_dev(const struct net_device *dev,
					   int hook_id)
{
	return false;
}
static inline enum vpnhide_bind_action
vpnhide_setsockopt_bind(struct sock *sk, int optname, sockptr_t optval,
			unsigned int optlen, union vpnhide_bind_snapshot *snap)
{
	return VPNHIDE_BIND_PASSTHROUGH;
}
static inline bool vpnhide_should_hide_dentry(const struct dentry *dentry,
					      int hook_id)
{
	return false;
}
static inline bool vpnhide_is_iface_listing_dir(const struct dentry *dentry)
{
	return false;
}

#endif /* CONFIG_VPNHIDE */
#endif /* _LINUX_VPNHIDE_H */
