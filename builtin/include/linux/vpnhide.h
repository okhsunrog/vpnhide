/* SPDX-License-Identifier: MIT */
/*
 * vpnhide — public in-tree API included by patched kernel subsystems.
 *
 * This is the compile-time (builtin) sibling of the out-of-tree kmod. The .ko
 * attaches the same filtering logic with kretprobes; here the logic is compiled
 * into vmlinux and reached through direct call-site hooks placed by the
 * per-version patches under builtin/versions/<kmi>/. No kprobes, no module
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
#include <linux/version.h>	/* LINUX_VERSION_CODE / KERNEL_VERSION */

/*
 * sockptr_t landed in 5.9; __sys_setsockopt took a sockptr optval from then on.
 * Pre-5.9 kernels (android11-5.4 and older) pass a bare `char __user *optval`
 * with set_fs(), so those call sites use vpnhide_setsockopt_bind_user() instead
 * and must not see the sockptr-typed declaration (the header would not even
 * compile — <linux/sockptr.h> does not exist there).
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(5, 9, 0)
#include <linux/sockptr.h>	/* sockptr_t */
#define VPNHIDE_HAVE_SOCKPTR 1
#endif

/*
 * struct fib_rt_info (the packed RTM_GETROUTE-reply descriptor) landed in 5.5.
 * Pre-5.5 kernels call fib_dump_info(fi, dst, dst_len, …) with the fields
 * spread out, so those sites use vpnhide_hide_fib_dump_raw() instead.
 */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(5, 5, 0)
#define VPNHIDE_HAVE_FIB_RT_INFO 1
#endif

struct net_device;
struct sock;
struct dentry;
struct file;
struct dir_context;
struct fib_info;
struct fib_rt_info;
struct fib6_info;
struct dst_entry;
struct fib_rule;

/*
 * Global hook ids (data/hooks.toml -> generated/hook_ids.h). The patch at each
 * kernel call site passes the id of the hook it stands in for, so the same few
 * predicates below preserve the .ko's per-hook enable masks AND per-hook stats.
 * Kept as a plain int in the public API to avoid pulling the generated enum
 * into every patched translation unit; security/vpnhide/ uses the real enum.
 *
 * These VPNHIDE_HID_* constants are what the call-site patches pass. Their
 * values mirror data/hooks.toml; core.c static_asserts each against the
 * generated VPNHIDE_HOOK_* enum, so a renumber cannot silently desync a patch.
 * (A distinct prefix avoids colliding with the generated enum in the driver.)
 */
#define VPNHIDE_HID_FIB_ROUTE_SEQ_SHOW	0
#define VPNHIDE_HID_IPV6_ROUTE_SEQ_SHOW	1
#define VPNHIDE_HID_RTNL_FILL_IFINFO	2
#define VPNHIDE_HID_INET_FILL_IFADDR	3
#define VPNHIDE_HID_INET6_FILL_IFADDR	4
#define VPNHIDE_HID_DEV_IOCTL		5
#define VPNHIDE_HID_SOCK_IOCTL		6
#define VPNHIDE_HID_FIB_DUMP_INFO	7
#define VPNHIDE_HID_RT6_FILL_NODE	8
#define VPNHIDE_HID_FIB_NL_FILL_RULE	9

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
 *
 * Two variants for the two eras of __sys_setsockopt: the sockptr form for 5.9+
 * (our GKI KMIs) and the bare-user-pointer form for pre-5.9 (android11-5.4 and
 * older), where the patch swaps optval under set_fs(KERNEL_DS) on FROZEN.
 */
#ifdef VPNHIDE_HAVE_SOCKPTR
enum vpnhide_bind_action vpnhide_setsockopt_bind(struct sock *sk, int optname,
						 sockptr_t optval,
						 unsigned int optlen,
						 union vpnhide_bind_snapshot *snap);
#endif
enum vpnhide_bind_action vpnhide_setsockopt_bind_user(struct sock *sk, int optname,
						      const char __user *optval,
						      unsigned int optlen,
						      union vpnhide_bind_snapshot *snap);

/*
 * Route / rule concealment (RTM_GETROUTE, /proc/net/route, RTM_GETRULE). Unlike
 * the interface predicates these carry their own decision — beyond hiding a VPN
 * interface's own routes they also drop a public host-route pinned to a physical
 * uplink (the /32 or /128 a VPN client installs to reach its server, which leaks
 * the server IP even when the tun is hidden) and the target UID's policy rule.
 * The per-version patch calls these at the top of each emit function and skips
 * the entry (return 0 / continue) on true. Dev extraction uses the kernel's own
 * fib_info_nhc()/nexthop_fib6_nh() accessors.
 */
bool vpnhide_hide_fib_route(const struct fib_info *fi);		/* fib_route_seq_show */
bool vpnhide_hide_fib6_route(struct fib6_info *rt);		/* ipv6_route_seq_show */
#ifdef VPNHIDE_HAVE_FIB_RT_INFO
bool vpnhide_hide_fib_dump(const struct fib_rt_info *fri);	/* fib_dump_info (5.5+) */
#else
bool vpnhide_hide_fib_dump_raw(const struct fib_info *fi, __be32 dst, int dst_len);
#endif
bool vpnhide_hide_rt6(struct fib6_info *rt, struct dst_entry *dst); /* rt6_fill_node */
bool vpnhide_hide_fib_rule(const struct fib_rule *rule);	/* fib_nl_fill_rule */

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
#ifdef VPNHIDE_HAVE_SOCKPTR
static inline enum vpnhide_bind_action
vpnhide_setsockopt_bind(struct sock *sk, int optname, sockptr_t optval,
			unsigned int optlen, union vpnhide_bind_snapshot *snap)
{
	return VPNHIDE_BIND_PASSTHROUGH;
}
#endif
static inline enum vpnhide_bind_action
vpnhide_setsockopt_bind_user(struct sock *sk, int optname,
			     const char __user *optval, unsigned int optlen,
			     union vpnhide_bind_snapshot *snap)
{
	return VPNHIDE_BIND_PASSTHROUGH;
}
static inline bool vpnhide_hide_fib_route(const struct fib_info *fi)
{
	return false;
}
static inline bool vpnhide_hide_fib6_route(struct fib6_info *rt)
{
	return false;
}
#ifdef VPNHIDE_HAVE_FIB_RT_INFO
static inline bool vpnhide_hide_fib_dump(const struct fib_rt_info *fri)
{
	return false;
}
#else
static inline bool vpnhide_hide_fib_dump_raw(const struct fib_info *fi,
					     __be32 dst, int dst_len)
{
	return false;
}
#endif
static inline bool vpnhide_hide_rt6(struct fib6_info *rt, struct dst_entry *dst)
{
	return false;
}
static inline bool vpnhide_hide_fib_rule(const struct fib_rule *rule)
{
	return false;
}

#endif /* CONFIG_VPNHIDE */

/*
 * Filesystem path concealment (optional). Kept in its own block so the stubs
 * cover BOTH CONFIG_VPNHIDE=n AND the CONFIG_VPNHIDE=y / CONFIG_VPNHIDE_FS_HIDING=n
 * case — otherwise a build with the network hooks but not the VFS hooks would
 * have no symbol for the patched fs call sites.
 */
#if defined(CONFIG_VPNHIDE) && defined(CONFIG_VPNHIDE_FS_HIDING)

/*
 * True if `dentry` resolves to a VPN interface's sysfs / proc-sys node that must
 * be concealed from the calling UID (lookup / open / getattr sites). Dentry-based
 * so it also covers relative openat, symlink following, and bind mounts. There is
 * one filesystem hook id, so no hook_id argument. On a true return the patch
 * releases its own resource (path_put / fput) and returns -ENOENT.
 */
bool vpnhide_should_hide_dentry(const struct dentry *dentry);

/*
 * iterate_dir per-entry filtering. If `file` lists interface nodes for a target
 * caller, vpnhide_readdir_begin() swaps in a filldir that drops VPN-interface
 * entries and returns true; the patch then calls the original iteration and
 * vpnhide_readdir_end() to restore the actor. Returns false (nothing swapped)
 * when the directory is not a listing dir or the caller is not targeted.
 */
bool vpnhide_readdir_begin(struct file *file, struct dir_context *ctx);
void vpnhide_readdir_end(struct dir_context *ctx);

#else /* !CONFIG_VPNHIDE || !CONFIG_VPNHIDE_FS_HIDING */

static inline bool vpnhide_should_hide_dentry(const struct dentry *dentry)
{
	return false;
}
static inline bool vpnhide_readdir_begin(struct file *file,
					 struct dir_context *ctx)
{
	return false;
}
static inline void vpnhide_readdir_end(struct dir_context *ctx)
{
}

#endif /* CONFIG_VPNHIDE && CONFIG_VPNHIDE_FS_HIDING */
#endif /* _LINUX_VPNHIDE_H */
