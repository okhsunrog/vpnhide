// SPDX-License-Identifier: MIT
/*
 * vpnhide (in-tree / builtin) — core: live config, interception stats, and the
 * folded /proc/vpnhide_ctl control+stats channel (docs/protocol.md). This is
 * the .ko's brain lifted verbatim; the only thing that changed is the ATTACH
 * mechanism (compile-time call-site hooks instead of kretprobes), so none of
 * the kretprobe/kallsyms/module machinery is here.
 *
 * The hook bodies in hook_socket.c / hook_netlink.c / hook_fs.c call
 * vpnhide_hook_active() + vpnhide_record_hook_hit() from here and the shared
 * vpnhide_iface_is_vpn() from shared/vpnhide_logic.h — the same filtering logic
 * the .ko uses.
 */

#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/version.h>
#include <linux/slab.h>
#include <linux/cred.h>
#include <linux/uidgid.h>
#include <linux/string.h>
#include <linux/spinlock.h>
#include <linux/uaccess.h>
#include <linux/seq_file.h>
#include <linux/proc_fs.h>
#include <linux/if.h>
#include <linux/nsproxy.h>
#include <linux/netdevice.h>

#include <linux/vpnhide.h>

#include "vpnhide_internal.h"
#include "generated/iface_lists.h"
#include "shared/vpnhide_logic.h"

#define is_vpn_ifname(name) vpnhide_iface_is_vpn(name)

/* static_assert reached <linux/build_bug.h> in 5.1; pre-5.1 kernels
 * (android10-4.19 / 4.14) lack it, so fall back to the C11 primitive. */
#ifndef static_assert
#define static_assert(expr, ...) _Static_assert(expr, #expr)
#endif

/* The public VPNHIDE_HID_* the call-site patches pass must equal the generated
 * VPNHIDE_HOOK_* ids; guard against a data/hooks.toml renumber desyncing them. */
static_assert(VPNHIDE_HID_FIB_ROUTE_SEQ_SHOW == VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW, "");
static_assert(VPNHIDE_HID_IPV6_ROUTE_SEQ_SHOW == VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW, "");
static_assert(VPNHIDE_HID_RTNL_FILL_IFINFO == VPNHIDE_HOOK_RTNL_FILL_IFINFO, "");
static_assert(VPNHIDE_HID_INET_FILL_IFADDR == VPNHIDE_HOOK_INET_FILL_IFADDR, "");
static_assert(VPNHIDE_HID_INET6_FILL_IFADDR == VPNHIDE_HOOK_INET6_FILL_IFADDR, "");
static_assert(VPNHIDE_HID_DEV_IOCTL == VPNHIDE_HOOK_DEV_IOCTL, "");
static_assert(VPNHIDE_HID_SOCK_IOCTL == VPNHIDE_HOOK_SOCK_IOCTL, "");
static_assert(VPNHIDE_HID_FIB_DUMP_INFO == VPNHIDE_HOOK_FIB_DUMP_INFO, "");
static_assert(VPNHIDE_HID_RT6_FILL_NODE == VPNHIDE_HOOK_RT6_FILL_NODE, "");
static_assert(VPNHIDE_HID_FIB_NL_FILL_RULE == VPNHIDE_HOOK_FIB_NL_FILL_RULE, "");

/* ------------------------------------------------------------------ */
/*  Debug flag (set from the config snapshot's `debug` line)          */
/* ------------------------------------------------------------------ */

bool vpnhide_debug_enabled;

/* ------------------------------------------------------------------ */
/*  Live config (control protocol §4.3)                               */
/*                                                                    */
/*  Per-target per-hook mask so the app can enable hooks individually; */
/*  a hook fires only when its bit is set for the calling UID. Written */
/*  via /proc/vpnhide_ctl; same struct + parser the .ko and KPM use    */
/*  (shared/vpnhide_logic.h).                                          */
/* ------------------------------------------------------------------ */

/* Parallel arrays, not an array of structs: the lookup touches only the uids,
 * so keeping them contiguous halves the cache lines a bisection walks. The
 * parser hands them over sorted ascending (protocol §4.3). */
static u32 target_uids[MAX_TARGET_UIDS];
static u32 target_masks[MAX_TARGET_UIDS];
static int nr_targets;
/* Hookmask for any uid NOT in target_uids (protocol §4.3 `default`). */
static u32 default_hookmask;
static DEFINE_SPINLOCK(targets_lock);
/* OR of every target's hookmask AND the default — a lock-free fast-path gate so
 * vpnhide_hook_active() rejects the common case (a hook enabled for nobody)
 * with one atomic-free read instead of taking targets_lock on every call.
 * Recomputed under targets_lock on each apply; a torn read only costs a brief
 * over/under-filter around a (rare) config change. */
static u32 active_hook_mask;

/*
 * First uid Android hands an ordinary app; everything below is a platform AID
 * (system_server 1000, radio 1001, shell 2000, the OEM 5000s). Compared on the
 * app-id so a secondary-profile uid (1010234 in profile 10) classifies the same
 * as 10234 in the owner profile.
 */
#define VPNHIDE_FIRST_APP_UID 10000
#define VPNHIDE_PER_USER_RANGE 100000

/* The enabled-hook mask for the calling UID. */
static u32 target_mask(void)
{
	uid_t uid = from_kuid(&init_user_ns, current_uid());
	u32 mask;
	int lo, hi;

	/*
	 * Below the app range a uid is a platform identity shared by many
	 * components; "hide from that app" is not expressible by UID there, and
	 * hiding from everything running as 1000 is how a device ends up
	 * believing it has no route. The floor is unconditional and not
	 * expressible in a config — it also keeps uid 0 honest for the app's
	 * root-differential diagnostics. NOT the same set as FLAG_SYSTEM:
	 * vendor-preinstalled apps keep ordinary 10xxx uids and stay targetable.
	 */
	if (uid % VPNHIDE_PER_USER_RANGE < VPNHIDE_FIRST_APP_UID)
		return 0;

	spin_lock(&targets_lock);
	mask = default_hookmask;
	lo = 0;
	hi = nr_targets - 1;
	while (lo <= hi) {
		int mid = lo + (hi - lo) / 2;

		if (target_uids[mid] == uid) {
			mask = target_masks[mid];
			break;
		}
		if (target_uids[mid] < uid)
			lo = mid + 1;
		else
			hi = mid - 1;
	}
	spin_unlock(&targets_lock);
	return mask;
}

/* True if `hook_id` is enabled for the calling UID (per-hook gate, §4.3).
 * Fast path: if no target enables this hook, skip the per-uid lock+search. */
bool vpnhide_hook_active(enum vpnhide_hook_id hook_id)
{
	u32 bit = vpnhide_hook_bit(hook_id);

	if (!(READ_ONCE(active_hook_mask) & bit))
		return false;
	return (target_mask() & bit) != 0;
}

/* ------------------------------------------------------------------ */
/*  Native interception stats (protocol §4.3 `stats`)                 */
/* ------------------------------------------------------------------ */

struct stats_row {
	uid_t uid;
	u64 counts[VPNHIDE_KMOD_STATS_HOOK_COUNT];
};

static struct stats_row stats_rows[MAX_TARGET_UIDS];
/* Sorted index by uid for binary lookup; rows stay in insertion order so adding
 * a uid shifts only this compact index, not the ~15 KiB stats table, and
 * seq_file iteration stays stable when another uid records its first hit. */
static int stats_uid_order[MAX_TARGET_UIDS];
static int nr_stats_rows;
static DEFINE_SPINLOCK(stats_lock);

/* stats_lock must be held. */
static struct stats_row *find_or_add_stats_row(uid_t uid)
{
	struct stats_row *row;
	int lo = 0;
	int hi = nr_stats_rows;
	int mid, row_index;

	while (lo < hi) {
		mid = lo + (hi - lo) / 2;
		row = &stats_rows[stats_uid_order[mid]];
		if (row->uid < uid)
			lo = mid + 1;
		else
			hi = mid;
	}

	if (lo < nr_stats_rows) {
		row = &stats_rows[stats_uid_order[lo]];
		if (row->uid == uid)
			return row;
	}
	if (nr_stats_rows >= MAX_TARGET_UIDS)
		return NULL;

	row_index = nr_stats_rows;
	memmove(&stats_uid_order[lo + 1], &stats_uid_order[lo],
		(nr_stats_rows - lo) * sizeof(stats_uid_order[0]));
	stats_uid_order[lo] = row_index;
	nr_stats_rows++;

	row = &stats_rows[row_index];
	row->uid = uid;
	memset(row->counts, 0, sizeof(row->counts));
	return row;
}

void vpnhide_record_hook_hit(enum vpnhide_hook_id hook_id)
{
	struct stats_row *row;
	uid_t uid;
	unsigned long flags;
	int hook_slot;

	hook_slot = vpnhide_kmod_stats_hook_slot(hook_id);
	if (hook_slot < 0)
		return;

	uid = from_kuid(&init_user_ns, current_uid());
	spin_lock_irqsave(&stats_lock, flags);
	row = find_or_add_stats_row(uid);
	if (row)
		row->counts[hook_slot]++;
	spin_unlock_irqrestore(&stats_lock, flags);
}

/* ------------------------------------------------------------------ */
/*  /proc/vpnhide_ctl — the folded control + stats channel            */
/* ------------------------------------------------------------------ */

/*
 * In-tree, every owned hook is a compiled-in call site: if CONFIG_VPNHIDE is
 * set they are all present, so there is no partial-registration failure mode
 * (unlike the .ko, where a kretprobe can fail to attach on an odd ROM). The
 * `hooks` mask is therefore constant and `error` is always OK.
 */
static u32 installed_hook_mask(void)
{
	return VPNHIDE_OWNED_HOOK_MASK;
}

static ssize_t ctl_write(struct file *file, const char __user *ubuf,
			 size_t count, loff_t *ppos)
{
	char *buf;
	struct vpnhide_target *newt;
	unsigned int default_mask = 0;
	int n, dbg;

	if (count > PAGE_SIZE)
		return -EINVAL;

	buf = kmalloc(count + 1, GFP_KERNEL);
	if (!buf)
		return -ENOMEM;
	newt = kcalloc(MAX_TARGET_UIDS, sizeof(*newt), GFP_KERNEL);
	if (!newt) {
		kfree(buf);
		return -ENOMEM;
	}

	if (copy_from_user(buf, ubuf, count)) {
		kfree(newt);
		kfree(buf);
		return -EFAULT;
	}
	buf[count] = '\0';

	/* Seed `dbg` with the live value so an absent `debug` line means
	 * "unchanged from current", per §4.3. */
	dbg = READ_ONCE(vpnhide_debug_enabled) ? 1 : 0;
	n = vpnhide_parse_config(buf, count, newt, MAX_TARGET_UIDS, &dbg,
				 &default_mask);
	kfree(buf);

	/* A payload with no valid header / wrong version / broken `end` fuse is
	 * rejected whole (§3) — loud -EINVAL, never a silent partial wipe. */
	if (n < 0) {
		kfree(newt);
		return -EINVAL;
	}

	spin_lock(&targets_lock);
	{
		const u32 owned_mask = VPNHIDE_OWNED_HOOK_MASK;
		u32 mask;
		int i;

		default_mask &= owned_mask;
		mask = default_mask;
		for (i = 0; i < n; i++) {
			target_uids[i] = newt[i].uid;
			target_masks[i] = newt[i].hookmask & owned_mask;
			mask |= target_masks[i];
		}
		nr_targets = n;
		default_hookmask = default_mask;
		WRITE_ONCE(active_hook_mask, mask);
	}
	spin_unlock(&targets_lock);
	kfree(newt);
	WRITE_ONCE(vpnhide_debug_enabled, dbg ? true : false);

	pr_info(MODNAME ": config applied — %d targets, debug=%d\n", n, dbg);
	return count;
}

/* Read side: banner, status, stats header, then one UID per seq_file record.
 * seq_file may replay a record when the userspace buffer fills, so output
 * streams without a whole-snapshot allocation or byte ceiling. */
static void *ctl_seq_start(struct seq_file *m, loff_t *pos)
{
	if (*pos >= 3 + MAX_TARGET_UIDS)
		return NULL;
	return (void *)(unsigned long)(*pos + 1);
}

static void *ctl_seq_next(struct seq_file *m, void *v, loff_t *pos)
{
	(*pos)++;
	return ctl_seq_start(m, pos);
}

static void ctl_seq_stop(struct seq_file *m, void *v)
{
}

static int ctl_seq_show(struct seq_file *m, void *v)
{
	unsigned long item = (unsigned long)v - 1;

	if (item == 0) {
		seq_puts(m, VPNHIDE_READ_BANNER);
	} else if (item == 1) {
		u32 hooks = installed_hook_mask();

		/* Every owned hook is compiled in, so error is unconditionally
		 * OK — see installed_hook_mask(). */
		seq_printf(m,
			   "vpnhide %u status\nbackend 0x%x\nkver 0x%x\n"
			   "hooks 0x%x\nerror 0x%x\n",
			   VPNHIDE_TELEMETRY_VERSION, VPNHIDE_BACKEND_BUILTIN,
			   LINUX_VERSION_CODE, hooks, VPNHIDE_ERR_OK);
	} else if (item == 2) {
		seq_printf(m, "vpnhide %u stats\n", VPNHIDE_TELEMETRY_VERSION);
	} else {
		struct stats_row row;
		unsigned long flags;
		int hook, index = (int)item - 3;

		spin_lock_irqsave(&stats_lock, flags);
		if (index >= nr_stats_rows) {
			spin_unlock_irqrestore(&stats_lock, flags);
			return 0;
		}
		row = stats_rows[index];
		spin_unlock_irqrestore(&stats_lock, flags);

		seq_printf(m, "0x%x", row.uid);
		for (hook = 0; hook < VPNHIDE_KMOD_STATS_HOOK_COUNT; hook++) {
			if (!row.counts[hook])
				continue;
			seq_printf(m, " 0x%x:0x%llx",
				   vpnhide_kmod_stats_hook_id(hook),
				   row.counts[hook]);
		}
		seq_putc(m, '\n');
	}
	return 0;
}

static const struct seq_operations ctl_seq_ops = {
	.start = ctl_seq_start,
	.next = ctl_seq_next,
	.stop = ctl_seq_stop,
	.show = ctl_seq_show,
};

static int ctl_open(struct inode *inode, struct file *file)
{
	return seq_open(file, &ctl_seq_ops);
}

/* struct proc_ops split out of file_operations in 5.6; pre-5.6 (android11-5.4
 * and older) still registers /proc handlers through file_operations. */
#if LINUX_VERSION_CODE >= KERNEL_VERSION(5, 6, 0)
static const struct proc_ops ctl_proc_ops = {
	.proc_open = ctl_open,
	.proc_read = seq_read,
	.proc_write = ctl_write,
	.proc_lseek = seq_lseek,
	.proc_release = seq_release,
};
#else
static const struct file_operations ctl_proc_ops = {
	.open = ctl_open,
	.read = seq_read,
	.write = ctl_write,
	.llseek = seq_lseek,
	.release = seq_release,
};
#endif

/* ------------------------------------------------------------------ */
/*  /proc/vpnhide_diag — read-only field-debugging dump               */
/*                                                                    */
/*  NOT part of the frozen control/telemetry wire: a separate proc     */
/*  node, plain human-readable text, never parsed by the activator or  */
/*  app. Reports the live is_vpn_ifname() verdict over every netdev in */
/*  the reader's netns — for root-causing an unexpected match hiding a  */
/*  non-VPN interface. Deliberately reports the CURRENT verdict as-is,  */
/*  bugs included. Unlike the .ko there are no kretprobe miss counters  */
/*  to show: in-tree hooks cannot miss.                                */
/* ------------------------------------------------------------------ */

static const char *netdev_operstate_str(unsigned char operstate)
{
	switch (operstate) {
	case IF_OPER_UNKNOWN:		return "unknown";
	case IF_OPER_NOTPRESENT:	return "notpresent";
	case IF_OPER_DOWN:		return "down";
	case IF_OPER_LOWERLAYERDOWN:	return "lowerlayerdown";
	case IF_OPER_TESTING:		return "testing";
	case IF_OPER_DORMANT:		return "dormant";
	case IF_OPER_UP:		return "up";
	}
	return "?";
}

static int vpnhide_diag_show(struct seq_file *m, void *v)
{
	struct nsproxy *nsproxy;
	struct net *net;
	struct net_device *dev;

	seq_puts(m, "vpnhide diag\n");
	seq_printf(m, "backend 0x%x active_hook_mask 0x%x installed_hook_mask 0x%x\n",
		   VPNHIDE_BACKEND_BUILTIN, READ_ONCE(active_hook_mask),
		   installed_hook_mask());

	/* Live is_vpn_ifname() verdict over every netdev in the reader's (root
	 * shell's) network namespace. Runs in process context off seq_read(),
	 * so rcu_read_lock() + for_each_netdev_rcu() is the ordinary safe walk. */
	rcu_read_lock();
	nsproxy = READ_ONCE(current->nsproxy);
	net = nsproxy ? nsproxy->net_ns : NULL;
	if (!net) {
		rcu_read_unlock();
		seq_puts(m, "iface (no net namespace available)\n");
		return 0;
	}
	for_each_netdev_rcu(net, dev) {
		char name[IFNAMSIZ];

		memcpy(name, dev->name, IFNAMSIZ);
		name[IFNAMSIZ - 1] = '\0';
		seq_printf(m,
			   "iface %s ifindex=%d is_vpn=%d up=%d operstate=%s flags=0x%x\n",
			   name, dev->ifindex, is_vpn_ifname(name) ? 1 : 0,
			   !!(dev->flags & IFF_UP),
			   netdev_operstate_str(dev->operstate), dev->flags);
	}
	rcu_read_unlock();

	return 0;
}

/* ------------------------------------------------------------------ */
/*  Init                                                              */
/* ------------------------------------------------------------------ */

static int __init vpnhide_init(void)
{
	/* 0600: root-only. The config snapshot is written here by service.sh
	 * and the VPN Hide app (both root). Apps must not see the control
	 * channel. Until a config is written, active_hook_mask stays 0 and
	 * every call-site hook is a no-op, so ordering against the net/fs
	 * subsystems that call into us is not load-bearing. */
	if (!proc_create("vpnhide_ctl", 0600, NULL, &ctl_proc_ops)) {
		pr_err(MODNAME ": proc_create(vpnhide_ctl) failed\n");
		return -ENOMEM;
	}

	/* Diagnostic-only node — best effort, 0400 root-only read. */
	if (!proc_create_single("vpnhide_diag", 0400, NULL, vpnhide_diag_show))
		pr_warn(MODNAME
			": proc_create_single(vpnhide_diag) failed — diagnostics unavailable\n");

	pr_info(MODNAME
		": in-tree backend ready — write a config snapshot to /proc/vpnhide_ctl\n");
	return 0;
}

/* late_initcall: procfs is up, and the call-site hooks are already safe (they
 * no-op until a config arrives). No exit path — the kernel is built with this. */
late_initcall(vpnhide_init);
