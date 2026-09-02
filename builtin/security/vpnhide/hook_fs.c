// SPDX-License-Identifier: MIT
/*
 * vpnhide (in-tree) — optional filesystem path concealment
 * (CONFIG_VPNHIDE_FS_HIDING). Conceals a VPN interface's sysfs / proc-sys nodes.
 *
 * The dentry-matching predicates are a verbatim port of the .ko's: matching by
 * resolved dentry (not a userspace pathname string) also covers relative openat,
 * symlink following, and bind mounts. What changed is only the attach: the .ko
 * redirected filename_lookup / do_filp_open / vfs_getattr / iterate_dir with
 * kprobes and had to resolve path_put through a throwaway kprobe; in-tree the
 * per-version patch sits at each call site and releases its own resource
 * (path_put / fput) directly, so none of that machinery is here.
 */

#include <linux/kernel.h>
#include <linux/string.h>
#include <linux/if.h>
#include <linux/dcache.h>
#include <linux/fs.h>
#include <linux/list.h>
#include <linux/spinlock.h>
#include <linux/slab.h>
#include <linux/version.h>

#include <linux/vpnhide.h>

#include "vpnhide_internal.h"
#include "generated/iface_lists.h"
#include "shared/vpnhide_logic.h"

#define is_vpn_ifname(name) vpnhide_iface_is_vpn(name)

static bool filesystem_filter_active(void)
{
	return vpnhide_hook_active(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
}

/* ------------------------------------------------------------------ */
/*  Dentry matching (verbatim from the .ko)                           */
/* ------------------------------------------------------------------ */

static bool qstr_equals(const struct qstr *name, const char *literal)
{
	size_t len = strlen(literal);

	return name && name->len == len && !memcmp(name->name, literal, len);
}

static bool qstr_is_vpn_iface(const struct qstr *name)
{
	char iface[IFNAMSIZ];

	if (!name || !name->len || name->len >= sizeof(iface))
		return false;
	memcpy(iface, name->name, name->len);
	iface[name->len] = '\0';
	return is_vpn_ifname(iface);
}

static bool dentry_is_proc_iface_path(struct dentry *vpn)
{
	struct dentry *kind, *family, *net, *sys;

	kind = READ_ONCE(vpn->d_parent);
	if (!kind || (!qstr_equals(&kind->d_name, "conf") &&
		      !qstr_equals(&kind->d_name, "neigh")))
		return false;
	family = READ_ONCE(kind->d_parent);
	if (!family || (!qstr_equals(&family->d_name, "ipv4") &&
			!qstr_equals(&family->d_name, "ipv6")))
		return false;
	net = READ_ONCE(family->d_parent);
	if (!net || !qstr_equals(&net->d_name, "net"))
		return false;
	sys = READ_ONCE(net->d_parent);
	return sys && qstr_equals(&sys->d_name, "sys");
}

static bool dentry_is_hidden_iface_path(const struct dentry *dentry)
{
	const struct file_system_type *type;
	struct dentry *cursor;
	int depth;

	if (!dentry || !dentry->d_sb || !dentry->d_sb->s_type)
		return false;
	type = dentry->d_sb->s_type;

	/* Only sysfs (the net interface dirs) and proc-sys (the per-iface
	 * /proc/sys/net dirs) host the interface nodes, so the walk below can
	 * only match on those two filesystems. Bail here for every other
	 * filesystem - the common case (ext4/f2fs/tmpfs) - instead of climbing
	 * every path's parent chain on each lookup/open/getattr while active. */
	if (strcmp(type->name, "sysfs") && strcmp(type->name, "proc"))
		return false;

	rcu_read_lock();
	cursor = (struct dentry *)dentry;
	for (depth = 0; cursor && depth < 16; depth++) {
		struct dentry *parent;

		if (!qstr_is_vpn_iface(&cursor->d_name))
			goto next;
		parent = READ_ONCE(cursor->d_parent);
		if (!strcmp(type->name, "sysfs") && parent &&
		    qstr_equals(&parent->d_name, "net")) {
			rcu_read_unlock();
			return true;
		}
		if (!strcmp(type->name, "proc") &&
		    dentry_is_proc_iface_path(cursor)) {
			rcu_read_unlock();
			return true;
		}
next:
		parent = READ_ONCE(cursor->d_parent);
		if (!parent || parent == cursor)
			break;
		cursor = parent;
	}
	rcu_read_unlock();
	return false;
}

static bool dentry_is_iface_listing_dir(const struct dentry *dentry)
{
	const struct file_system_type *type;
	struct dentry *family, *net, *sys;

	if (!dentry || !dentry->d_sb || !dentry->d_sb->s_type)
		return false;
	type = dentry->d_sb->s_type;

	rcu_read_lock();
	if (!strcmp(type->name, "sysfs") &&
	    qstr_equals(&dentry->d_name, "net")) {
		rcu_read_unlock();
		return true;
	}
	if (strcmp(type->name, "proc") ||
	    (!qstr_equals(&dentry->d_name, "conf") &&
	     !qstr_equals(&dentry->d_name, "neigh")))
		goto no;
	family = READ_ONCE(dentry->d_parent);
	if (!family || (!qstr_equals(&family->d_name, "ipv4") &&
			!qstr_equals(&family->d_name, "ipv6")))
		goto no;
	net = READ_ONCE(family->d_parent);
	if (!net || !qstr_equals(&net->d_name, "net"))
		goto no;
	sys = READ_ONCE(net->d_parent);
	if (sys && qstr_equals(&sys->d_name, "sys")) {
		rcu_read_unlock();
		return true;
	}
no:
	rcu_read_unlock();
	return false;
}

/* ------------------------------------------------------------------ */
/*  lookup / open / getattr predicate                                 */
/* ------------------------------------------------------------------ */

bool vpnhide_should_hide_dentry(const struct dentry *dentry)
{
	if (!filesystem_filter_active() || !dentry)
		return false;
	if (!dentry_is_hidden_iface_path(dentry))
		return false;
	vpnhide_record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
	return true;
}

/* ------------------------------------------------------------------ */
/*  iterate_dir per-entry filtering                                    */
/*                                                                    */
/*  Keyed on the dir_context because the caller allocates it and we    */
/*  cannot extend it — same design as the .ko. The patch calls         */
/*  vpnhide_readdir_begin() before the real iteration and              */
/*  vpnhide_readdir_end() after.                                       */
/* ------------------------------------------------------------------ */

struct readdir_filter_state {
	struct list_head node;
	struct dir_context *ctx;
	filldir_t original_actor;
};

static LIST_HEAD(readdir_filter_states);
static DEFINE_SPINLOCK(readdir_filter_lock);

#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
static bool
#define VPNHIDE_FILLDIR_CONTINUE true
#else
static int
#define VPNHIDE_FILLDIR_CONTINUE 0
#endif
vpnhide_filldir(struct dir_context *ctx, const char *name, int namelen,
		loff_t offset, u64 ino, unsigned int d_type)
{
	struct readdir_filter_state *state;
	filldir_t actor = NULL;
	char iface[IFNAMSIZ];
	unsigned long irqflags;

	spin_lock_irqsave(&readdir_filter_lock, irqflags);
	list_for_each_entry(state, &readdir_filter_states, node) {
		if (state->ctx == ctx) {
			actor = state->original_actor;
			break;
		}
	}
	spin_unlock_irqrestore(&readdir_filter_lock, irqflags);

	if (namelen > 0 && namelen < sizeof(iface)) {
		memcpy(iface, name, namelen);
		iface[namelen] = '\0';
		if (is_vpn_ifname(iface)) {
			vpnhide_record_hook_hit(
				VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
			return VPNHIDE_FILLDIR_CONTINUE;
		}
	}
	return actor ? actor(ctx, name, namelen, offset, ino, d_type) :
		       VPNHIDE_FILLDIR_CONTINUE;
}

bool vpnhide_readdir_begin(struct file *file, struct dir_context *ctx)
{
	struct readdir_filter_state *state;
	unsigned long irqflags;

	if (!filesystem_filter_active() || !file || !ctx ||
	    !dentry_is_iface_listing_dir(file->f_path.dentry))
		return false;

	state = kmalloc(sizeof(*state), GFP_KERNEL);
	if (!state)
		return false;
	state->ctx = ctx;
	state->original_actor = READ_ONCE(ctx->actor);
	spin_lock_irqsave(&readdir_filter_lock, irqflags);
	list_add(&state->node, &readdir_filter_states);
	spin_unlock_irqrestore(&readdir_filter_lock, irqflags);
	WRITE_ONCE(ctx->actor, vpnhide_filldir);
	return true;
}

void vpnhide_readdir_end(struct dir_context *ctx)
{
	struct readdir_filter_state *state, *tmp;
	unsigned long irqflags;

	spin_lock_irqsave(&readdir_filter_lock, irqflags);
	list_for_each_entry_safe(state, tmp, &readdir_filter_states, node) {
		if (state->ctx != ctx)
			continue;
		WRITE_ONCE(ctx->actor, state->original_actor);
		list_del(&state->node);
		spin_unlock_irqrestore(&readdir_filter_lock, irqflags);
		kfree(state);
		return;
	}
	spin_unlock_irqrestore(&readdir_filter_lock, irqflags);
}
