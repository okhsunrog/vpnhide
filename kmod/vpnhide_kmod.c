// SPDX-License-Identifier: MIT
/*
 * vpnhide_kmod — kernel module that hides VPN network interfaces from
 * selected Android apps by filtering ioctl, netlink, and procfs responses and
 * refusing VPN-interface socket binds based on the calling process's UID.
 *
 * Uses kretprobes for return-value/data filtering and ordinary entry kprobes
 * for pre-mutation socket-bind replacement, so no modification of the running
 * kernel is needed; works on stock Android GKI kernels with CONFIG_KPROBES=y.
 *
 * Hooks:
 *   - dev_ioctl: filters SIOCGIFFLAGS / SIOCGIFNAME / SIOCGIFMTU / etc.
 *   - sock_ioctl: filters SIOCGIFCONF interface enumeration
 *   - rtnl_fill_ifinfo: filters RTM_NEWLINK netlink dumps (getifaddrs)
 *   - inet6_fill_ifaddr: filters RTM_GETADDR IPv6 responses (getifaddrs)
 *   - inet_fill_ifaddr: filters RTM_GETADDR IPv4 responses (getifaddrs)
 *   - fib_route_seq_show: filters /proc/net/route entries
 *   - ipv6_route_seq_show: filters /proc/net/ipv6_route entries
 *   - fib_dump_info: filters IPv4 RTM_GETROUTE dump replies
 *   - rt6_fill_node: filters IPv6 RTM_GETROUTE replies
 *   - fib_nl_fill_rule: filters policy routing rules for target UIDs
 *   - sock_setsockopt / sk_setsockopt: denies SO_BINDTODEVICE and
 *     SO_BINDTOIFINDEX for hidden VPN interfaces before socket state changes
 *
 * Control plane: a single folded node /proc/vpnhide_ctl carries the shared
 * control/stats protocol (docs/protocol.md). A write is a `vpnhide 2 config`
 * snapshot (per-UID hook mask + debug flag); a read returns the backend
 * `status` + `stats`. This replaces the old /proc/vpnhide_targets (decimal UID
 * list) and /proc/vpnhide_debug nodes — the same wire format every backend now
 * speaks (parser shared verbatim with the KPM via shared/vpnhide_logic.h).
 *
 * Architecture: arm64 only. Several handlers read function arguments via
 * `regs->regs[N]`, and the socket-bind entry probes redirect the saved PC
 * according to AAPCS64. On other architectures those registers have a
 * different meaning, so the build is gated below.
 *
 * Lifecycle: the module is intentionally non-unloadable. Root module managers
 * install, update, disable, and remove it across a reboot; keeping module text
 * resident for the whole boot also makes redirected entry-kprobe targets an
 * unconditional lifetime invariant.
 */

#include <linux/module.h>
#include <linux/moduleparam.h>
#include <linux/kernel.h>
#include <linux/version.h>
#include <linux/kprobes.h>
#include <linux/slab.h>
#include <linux/cred.h>
#include <linux/uidgid.h>
#include <linux/string.h>
#include <linux/net.h>
#include <linux/if.h>
#include <linux/uaccess.h>
#include <linux/seq_file.h>
#include <linux/proc_fs.h>
#include <linux/fs.h>
#include <linux/namei.h>
#include <linux/file.h>
#include <linux/list.h>
#include <linux/nsproxy.h>
#include <linux/netdevice.h>
#include <linux/rtnetlink.h>
#include <linux/skbuff.h>
#include <linux/inetdevice.h>
#include <net/sock.h>
#include <net/if_inet6.h>
#include <net/ip_fib.h>
#include <net/nexthop.h>
#include <net/ip6_fib.h>
#include <net/ip6_route.h>
#include <net/route.h>
#include <net/fib_rules.h>

#include "generated/iface_lists.h"
#include "generated/hook_ids.h"
#include "shared/vpnhide_logic.h"

#ifndef CONFIG_ARM64
#error "vpnhide_kmod currently supports only arm64 (handlers read regs->regs[N] directly)"
#endif

#define MODNAME "vpnhide"
/* Mirror of vpnhide_protocol::MAX_TARGET_UIDS (crates/protocol/src/lib.rs); the
 * activator truncates the projected config to this many targets, so keep both in
 * sync. */
#define MAX_TARGET_UIDS 160

/*
 * Pre-allocated kretprobe instance pool size, applied to every probe.
 * Default kernel `register_kretprobe` falls back to NR_CPUS*2 (≈ 18 on
 * a 9-core Pixel 8 Pro), which is too low for hot ioctl/netlink paths
 * under multi-app concurrency — exhausted pool causes silent
 * `nmissed++` and the return handler skipped, which surfaces as a VPN
 * iface leaking through a single probe call.
 *
 * 64 covers a comfortable working set (apps × threads doing
 * getifaddrs/SIOCGIFCONF/route reads at once) without burning
 * meaningful memory: 10 probes × 64 instances × ~80 B ≈ 50 KB total.
 */
#define VPNHIDE_KRETPROBE_MAXACTIVE 64

/* ------------------------------------------------------------------ */
/*  Debug logging — folded into the /proc/vpnhide_ctl config snapshot  */
/* ------------------------------------------------------------------ */

static bool debug_enabled;
static bool filesystem_hiding;
module_param(filesystem_hiding, bool, 0400);
MODULE_PARM_DESC(filesystem_hiding,
		 "install optional sysfs/proc-sys interface concealment hooks");

/*
 * `debug_enabled` is a single bool, set from the `debug` line of a config
 * snapshot written to /proc/vpnhide_ctl and read from every probe handler.
 * Use READ_ONCE/WRITE_ONCE so the compiler doesn't tear the access or hoist
 * it across the probe-hot path — kosher kernel style for unsynchronised flags.
 */
#define vpnhide_dbg(fmt, ...)                                     \
	do {                                                      \
		if (READ_ONCE(debug_enabled))                     \
			pr_info(MODNAME ": " fmt, ##__VA_ARGS__); \
	} while (0)

/* ------------------------------------------------------------------ */
/*  VPN interface name matching — see data/interfaces.toml            */
/* ------------------------------------------------------------------ */

#define is_vpn_ifname(name) vpnhide_iface_is_vpn(name)

/* ------------------------------------------------------------------ */
/*  Live config (control protocol §4.3)                               */
/*                                                                    */
/*  Each target carries a per-hook mask so the app can enable hooks   */
/*  individually; a hook fires only when its bit is set for the       */
/*  calling UID. Written via /proc/vpnhide_ctl; the same `vpnhide      */
/*  target` struct + parser the KPM uses (shared/vpnhide_logic.h).    */
/* ------------------------------------------------------------------ */

/* Parallel arrays, not an array of structs: the lookup below touches only the
 * uids, so keeping them contiguous halves the cache lines a search walks
 * (a full 160-uid set is one 640-byte run). The parser hands them over sorted
 * ascending (protocol §4.3), which is what lets the search be a bisection. */
static u32 target_uids[MAX_TARGET_UIDS];
static u32 target_masks[MAX_TARGET_UIDS];
static int nr_targets;
/* Hookmask for any uid NOT in target_uids (protocol §4.3 `default`). Zero — the
 * shipped blacklist — makes the array the set to act on; non-zero inverts that
 * and makes it the exception list. */
static u32 default_hookmask;
static DEFINE_SPINLOCK(targets_lock);
/* OR of every target's hookmask AND the default — a lock-free fast-path gate so
 * hook_active() can reject the common case (a hook enabled for nobody, e.g. no
 * targets yet) with a single atomic-free read instead of acquiring targets_lock
 * on every hooked syscall. Recomputed under targets_lock on each config apply; a
 * torn read only costs a brief over- or under-filter around a (rare) config
 * change. Mirrors the KPM's active_hook_mask. */
static u32 active_hook_mask;

/*
 * First uid Android hands to an ordinary app; everything below is a system AID
 * (system_server 1000, radio 1001, network_stack 1073, shell 2000, the OEM
 * 5000s). Compared on the app-id so a uid from a secondary profile — 1010234 in
 * profile 10 — classifies the same as 10234 in the owner profile.
 */
#define VPNHIDE_FIRST_APP_UID 10000
#define VPNHIDE_PER_USER_RANGE 100000

/* The enabled-hook mask for the calling UID. */
static u32 target_mask(void)
{
	uid_t uid = from_kuid(&init_user_ns, current_uid());
	u32 mask;
	int lo, hi;

	/* Below the app range a uid is not an app, it is a platform identity
	 * shared by many components: an app declaring sharedUserId
	 * "android.uid.system" resolves to 1000, the same uid as system_server.
	 * Since UID is the targeting key (§4.3), "hide from that app" is not
	 * expressible there — the nearest thing the wire can say is "hide from
	 * everything running as 1000", which is not what anyone asked for and
	 * is how a device ends up believing it has no route. Note this is NOT
	 * the same set as FLAG_SYSTEM: vendor-preinstalled apps keep ordinary
	 * 10xxx uids and stay targetable.
	 *
	 * So the floor is unconditional and deliberately not expressible in a
	 * config — neither a target nor a `default` can lift it. It also keeps
	 * uid 0 honest, which is what the app's root-differential diagnostics
	 * use as ground truth. Cheap besides: system daemons hit these syscalls
	 * constantly and now bail on one compare. The activator applies the
	 * same rule when projecting, so this is the backstop, not the only gate.
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
 * The .ko owns the full kernel hook mask, so it never masks foreign bits.
 * Fast path: if no target enables this hook, skip the per-uid lock+search. */
static bool hook_active(enum vpnhide_hook_id hook_id)
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
/* Sorted by stats_rows[index].uid for binary lookup. Rows themselves stay in
 * insertion order so adding a UID shifts at most this compact index (640
 * bytes), not the roughly 15 KiB stats table, while seq_file iteration remains
 * stable when another UID records its first hit concurrently. */
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

static void record_hook_hit(enum vpnhide_hook_id hook_id)
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

/* Forward decl: the probe registration table drives the `status` hooks mask. */
static u32 installed_hook_mask(void);

static u32 expected_hook_mask(void)
{
	u32 mask = VPNHIDE_KERNEL_HOOK_MASK;

	if (READ_ONCE(filesystem_hiding))
		mask |= vpnhide_hook_bit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
	return mask;
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
	dbg = READ_ONCE(debug_enabled) ? 1 : 0;
	n = vpnhide_parse_config(buf, count, newt, MAX_TARGET_UIDS, &dbg,
				 &default_mask);
	kfree(buf);

	/* A payload with no valid header / the wrong version / a broken `end`
	 * fuse is rejected whole (§3) — a loud -EINVAL, never a silent partial
	 * wipe. */
	if (n < 0) {
		kfree(newt);
		return -EINVAL;
	}

	spin_lock(&targets_lock);
	{
		const u32 owned_mask = VPNHIDE_KERNEL_HOOK_MASK |
				       VPNHIDE_KMOD_HOOK_MASK;
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
	WRITE_ONCE(debug_enabled, dbg ? true : false);

	pr_info(MODNAME ": config applied — %d targets, debug=%d\n", n, dbg);
	return count;
}

/* Read side: banner, status, stats header, then one UID per seq_file record.
 * seq_file may replay a record when its current userspace read buffer fills,
 * so the output streams without a whole-snapshot allocation or byte ceiling. */
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
		u32 expected_hooks = expected_hook_mask();
		u32 error = (hooks & expected_hooks) == expected_hooks ?
				    VPNHIDE_ERR_OK :
				    VPNHIDE_ERR_PARTIAL_HOOKS;

		seq_printf(m,
			   "vpnhide %u status\nbackend 0x%x\nkver 0x%x\n"
			   "hooks 0x%x\nerror 0x%x\n",
			   VPNHIDE_TELEMETRY_VERSION, VPNHIDE_BACKEND_KMOD,
			   LINUX_VERSION_CODE, hooks, error);
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

static const struct proc_ops ctl_proc_ops = {
	.proc_open = ctl_open,
	.proc_read = seq_read,
	.proc_write = ctl_write,
	.proc_lseek = seq_lseek,
	.proc_release = seq_release,
};

/* ================================================================== */
/*  Hook 1: dev_ioctl — all per-interface ioctls                      */
/*                                                                    */
/*  dev_ioctl() on GKI 6.1:                                          */
/*    int dev_ioctl(struct net *net, unsigned int cmd,                */
/*                  struct ifreq *ifr, void __user *data,            */
/*                  bool *need_copyout)                               */
/*  arm64: x0=net, x1=cmd, x2=ifr (KERNEL ptr), x3=data (__user)   */
/*                                                                    */
/*  Covers SIOCGIFFLAGS, SIOCGIFNAME, SIOCGIFMTU, SIOCGIFINDEX,     */
/*  SIOCGIFHWADDR, SIOCGIFADDR, and any other cmd that goes through  */
/*  dev_ioctl with a VPN interface name in ifr_name. Returns ENODEV  */
/*  for all of them.                                                  */
/*                                                                    */
/*  Note: SIOCGIFCONF goes through sock_ioctl -> dev_ifconf, not     */
/*  through dev_ioctl, so it is not covered here.                    */
/* ================================================================== */

struct dev_ioctl_data {
	unsigned int cmd;
	struct ifreq *kifr; /* kernel pointer, saved from x2 */
	bool active; /* true = caller is target UID, run ret handler */
};

static int dev_ioctl_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct dev_ioctl_data *data = (void *)ri->data;

	data->cmd = (unsigned int)regs->regs[1];
	data->kifr = (struct ifreq *)regs->regs[2];
	data->active = hook_active(VPNHIDE_HOOK_DEV_IOCTL);

	vpnhide_dbg("dev_ioctl_entry: uid=%u target=%d cmd=0x%x\n",
		    from_kuid(&init_user_ns, current_uid()), data->active,
		    data->cmd);
	return 0;
}

static int dev_ioctl_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct dev_ioctl_data *data = (void *)ri->data;
	char name[IFNAMSIZ];

	if (!data->active || regs_return_value(regs) != 0)
		return 0;

	/*
	 * ifr (x2) is a KERNEL pointer — the caller already did
	 * copy_from_user into a stack-local ifreq. Read via direct
	 * dereference; copy_from_user would EFAULT under ARM64 PAN.
	 */
	if (!data->kifr)
		return 0;

	memcpy(name, data->kifr->ifr_name, IFNAMSIZ);
	name[IFNAMSIZ - 1] = '\0';

	if (is_vpn_ifname(name)) {
		vpnhide_dbg("dev_ioctl_ret: hiding iface=%s cmd=0x%x\n", name,
			    data->cmd);
		regs_set_return_value(regs, -ENODEV);
		record_hook_hit(VPNHIDE_HOOK_DEV_IOCTL);
	}

	return 0;
}

static struct kretprobe dev_ioctl_krp = {
	.handler = dev_ioctl_ret,
	.entry_handler = dev_ioctl_entry,
	.data_size = sizeof(struct dev_ioctl_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "dev_ioctl",
};

/* ================================================================== */
/*  Hook 11: SO_BINDTODEVICE / SO_BINDTOIFINDEX                       */
/*                                                                    */
/*  A return-only kretprobe would be wrong here: by return time the   */
/*  kernel has already changed sk_bound_dev_if, so merely reporting   */
/*  ENODEV leaves a usable bound socket behind. A kprobe entry handler */
/*  cannot solve this safely either: it runs in atomic context, where */
/*  a not-yet-resident userspace option page cannot be faulted in.     */
/*                                                                    */
/*  These functions therefore use an ordinary entry kprobe's supported */
/*  execution-path redirection: its atomic pre_handler only changes PC */
/*  and returns !0. The replacement wrapper then runs after exception  */
/*  handling has finished, in the original process context, where      */
/*  copy_from_sockptr may fault normally. It snapshots the option once */
/*  and passes an immutable KERNEL_SOCKPTR to the original function,   */
/*  eliminating userspace TOCTOU. A hidden interface returns ENODEV    */
/*  without ever calling the mutation path.                            */
/*                                                                    */
/*  sock_setsockopt is the ordinary SOL_SOCKET path on supported GKI  */
/*  kernels. On 6.1+ MPTCP can delegate directly to sk_setsockopt, so */
/*  symbol is covered as a second required registration there.       */
/* ================================================================== */

union socket_bind_snapshot {
	char name[IFNAMSIZ];
	int ifindex;
};

enum socket_bind_action {
	VPNHIDE_BIND_PASSTHROUGH,
	VPNHIDE_BIND_FROZEN,
	VPNHIDE_BIND_DENY,
	VPNHIDE_BIND_FAULT,
};

/* Human-readable action for the debug trace. Never returns NULL. */
static const char *socket_bind_action_str(enum socket_bind_action action)
{
	switch (action) {
	case VPNHIDE_BIND_PASSTHROUGH:
		return "passthrough";
	case VPNHIDE_BIND_FROZEN:
		return "frozen";
	case VPNHIDE_BIND_DENY:
		return "deny";
	case VPNHIDE_BIND_FAULT:
		return "fault";
	}
	return "?";
}

typedef struct net_device *(*dev_get_by_index_rcu_fn)(struct net *net,
						      int ifindex);

/*
 * dev_get_by_index_rcu() is exported in kallsyms but, like path_put() above,
 * is absent from some OEMs' trimmed GKI KMI module symbol table — a Xiaomi
 * HyperOS android12-5.10 build reports "Unknown symbol dev_get_by_index_rcu"
 * and refuses to load the whole .ko, taking down the core VPN-hiding hooks
 * that never touch this bind path. Resolve it at load time through a throwaway
 * kprobe (kprobes walk kallsyms, not the module export table) so the .ko links
 * with no hard reference, and call it only through the __nocfi trampoline
 * below. Stays NULL on kernels that neither export nor expose it, in which case
 * classify_bind_ifindex() fails closed.
 */
static dev_get_by_index_rcu_fn resolved_dev_get_by_index_rcu;

/* __nocfi: resolved_dev_get_by_index_rcu holds a kallsyms address recovered
 * through a kprobe, which is not a CFI jump-table entry — a checked indirect
 * call to it faults on android12/13-era jump-table CFI kernels, so bypass the
 * check the same way the VFS redirect originals do. Caller holds rcu_read_lock;
 * the returned net_device is only valid within that section. */
static noinline __nocfi struct net_device *
call_resolved_dev_get_by_index_rcu(struct net *net, int ifindex)
{
	struct net_device *dev = resolved_dev_get_by_index_rcu(net, ifindex);

	barrier();
	return dev;
}

/* 1 = VPN interface, 0 = physical/non-VPN, -1 = unknown. Unknown positive
 * indexes fail closed: sock_bindtoindex accepts a non-existent positive index,
 * which would otherwise leave observable state on the socket. An unresolved
 * dev_get_by_index_rcu (NULL pointer) also falls through to the fail-closed
 * unknown path rather than dereferencing it. */
static int classify_bind_ifindex(struct sock *sk, int ifindex)
{
	struct net_device *dev;
	int result = -1;

	if (!sk || ifindex <= 0)
		return ifindex == 0 ? 0 : -1;
	if (!resolved_dev_get_by_index_rcu)
		return -1;
	rcu_read_lock();
	dev = call_resolved_dev_get_by_index_rcu(sock_net(sk), ifindex);
	if (dev)
		result = is_vpn_ifname(dev->name) ? 1 : 0;
	rcu_read_unlock();
	return result;
}

static enum socket_bind_action
prepare_socket_bind(struct sock *sk, int optname, sockptr_t optval,
		    unsigned int optlen, union socket_bind_snapshot *snapshot)
{
	/* No `level` gate on purpose. Both hooked functions (sock_setsockopt,
	 * sk_setsockopt) ARE the SOL_SOCKET option handler — reaching them proves
	 * level == SOL_SOCKET by control flow. We must NOT read the ABI `level`
	 * argument: sk_setsockopt never uses it, so whole-kernel LTO elides setting
	 * it at the direct __sys_setsockopt -> sk_setsockopt call (the wrapper then
	 * sees garbage, e.g. 0, and a `level != SOL_SOCKET` check would wrongly pass
	 * the bind through — the SO_BINDTODEVICE leak on LTO GKI builds where
	 * sock_setsockopt is inlined away). Gate on the hook toggle alone. */
	if (!hook_active(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE))
		return VPNHIDE_BIND_PASSTHROUGH;
	if (optname != SO_BINDTODEVICE && optname != SO_BINDTOIFINDEX)
		return VPNHIDE_BIND_PASSTHROUGH;

	memset(snapshot, 0, sizeof(*snapshot));
	if (optname == SO_BINDTODEVICE) {
		size_t n;

		/* sock_setbindtodevice takes an int and rejects this before
		 * touching optval. Preserve EINVAL and avoid needless uaccess. */
		if ((int)optlen < 0)
			return VPNHIDE_BIND_PASSTHROUGH;
		n = min_t(size_t, optlen, IFNAMSIZ - 1);

		if (n && copy_from_sockptr(snapshot->name, optval, n))
			return VPNHIDE_BIND_FAULT;
		if (snapshot->name[0] && is_vpn_ifname(snapshot->name))
			return VPNHIDE_BIND_DENY;
		return VPNHIDE_BIND_FROZEN;
	}

	if (optlen < sizeof(snapshot->ifindex))
		return VPNHIDE_BIND_PASSTHROUGH;
	if (copy_from_sockptr(&snapshot->ifindex, optval,
			      sizeof(snapshot->ifindex)))
		return VPNHIDE_BIND_FAULT;
	if (snapshot->ifindex > 0 &&
	    classify_bind_ifindex(sk, snapshot->ifindex) != 0)
		return VPNHIDE_BIND_DENY;
	return VPNHIDE_BIND_FROZEN;
}

typedef int (*sock_setsockopt_fn)(struct socket *, int, int, sockptr_t,
				  unsigned int);
static sock_setsockopt_fn original_sock_setsockopt;

#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
typedef int (*sk_setsockopt_fn)(struct sock *, int, int, sockptr_t,
				unsigned int);
static sk_setsockopt_fn original_sk_setsockopt;
#endif

/* noinline + a post-call barrier keep the original call from becoming a tail
 * branch: the redirect kprobe uses the module return address to distinguish it
 * from an external call that must be redirected. __nocfi is required for the
 * non-exported sk_setsockopt address resolved through kprobe metadata. */
static noinline __nocfi int
call_original_sock_setsockopt(struct socket *sock, int level, int optname,
			      sockptr_t optval, unsigned int optlen)
{
	int ret =
		original_sock_setsockopt(sock, level, optname, optval, optlen);

	barrier();
	return ret;
}

static noinline int vpnhide_sock_setsockopt(struct socket *sock, int level,
					    int optname, sockptr_t optval,
					    unsigned int optlen)
{
	union socket_bind_snapshot snapshot;
	enum socket_bind_action action =
		prepare_socket_bind(sock ? READ_ONCE(sock->sk) : NULL, optname,
				    optval, optlen, &snapshot);

	vpnhide_dbg(
		"sock_setsockopt_entry: uid=%u level=%d optname=%d action=%s\n",
		from_kuid(&init_user_ns, current_uid()), level, optname,
		socket_bind_action_str(action));

	if (action == VPNHIDE_BIND_DENY) {
		record_hook_hit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
		return -ENODEV;
	}
	if (action == VPNHIDE_BIND_FAULT)
		return -EFAULT;
	if (action == VPNHIDE_BIND_FROZEN)
		optval = KERNEL_SOCKPTR(&snapshot);
	return call_original_sock_setsockopt(sock, level, optname, optval,
					     optlen);
}

#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
static noinline __nocfi int call_original_sk_setsockopt(struct sock *sk,
							int level, int optname,
							sockptr_t optval,
							unsigned int optlen)
{
	int ret = original_sk_setsockopt(sk, level, optname, optval, optlen);

	barrier();
	return ret;
}

static noinline int vpnhide_sk_setsockopt(struct sock *sk, int level,
					  int optname, sockptr_t optval,
					  unsigned int optlen)
{
	union socket_bind_snapshot snapshot;
	enum socket_bind_action action =
		prepare_socket_bind(sk, optname, optval, optlen, &snapshot);

	vpnhide_dbg(
		"sk_setsockopt_entry: uid=%u level=%d optname=%d action=%s\n",
		from_kuid(&init_user_ns, current_uid()), level, optname,
		socket_bind_action_str(action));

	if (action == VPNHIDE_BIND_DENY) {
		record_hook_hit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
		return -ENODEV;
	}
	if (action == VPNHIDE_BIND_FAULT)
		return -EFAULT;
	if (action == VPNHIDE_BIND_FROZEN)
		optval = KERNEL_SOCKPTR(&snapshot);
	return call_original_sk_setsockopt(sk, level, optname, optval, optlen);
}
#endif

struct socket_bind_kprobe_hook {
	const char *name;
	unsigned long replacement;
	struct kprobe kp;
	bool registered;
};

static bool socket_bind_hooks_ready;

static int socket_bind_kprobe_pre(struct kprobe *kp, struct pt_regs *regs)
{
	struct socket_bind_kprobe_hook *hook =
		container_of(kp, struct socket_bind_kprobe_hook, kp);
	unsigned long parent_ip = regs->regs[30];

	if (!READ_ONCE(socket_bind_hooks_ready) ||
	    within_module(parent_ip, THIS_MODULE))
		return 0;
	instruction_pointer_set(regs, hook->replacement);
	/* Per Documentation/trace/kprobes.rst, !0 tells kprobes not to
	 * single-step the replaced first instruction and to resume at our PC. */
	return 1;
}

/* A post_handler keeps these probes out of the optimized-kprobe detour path,
 * where changing PC from the pre_handler is not supported. It is reached only
 * for the module's intentional call-through to the original function. */
static void socket_bind_kprobe_post(struct kprobe *kp, struct pt_regs *regs,
				    unsigned long flags)
{
	(void)kp;
	(void)regs;
	(void)flags;
}

static struct socket_bind_kprobe_hook socket_bind_kprobe_hooks[] = {
	{
		.name = "sock_setsockopt",
		.replacement = (unsigned long)vpnhide_sock_setsockopt,
		.kp = {
			.symbol_name = "sock_setsockopt",
			.pre_handler = socket_bind_kprobe_pre,
			.post_handler = socket_bind_kprobe_post,
		},
	},
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
	{
		.name = "sk_setsockopt",
		.replacement = (unsigned long)vpnhide_sk_setsockopt,
		.kp = {
			.symbol_name = "sk_setsockopt",
			.pre_handler = socket_bind_kprobe_pre,
			.post_handler = socket_bind_kprobe_post,
		},
	},
#endif
};

static bool socket_bind_hooks_registered;

/* Registration keeps redirection disabled until every required probe exists.
 * Therefore rollback cannot race a redirected wrapper and needs no runtime
 * drain protocol. Once ready becomes true the permanent module keeps these
 * probes and their replacement text resident until reboot. */
static void rollback_socket_bind_hooks(void)
{
	int i;

	for (i = ARRAY_SIZE(socket_bind_kprobe_hooks) - 1; i >= 0; i--) {
		struct socket_bind_kprobe_hook *hook =
			&socket_bind_kprobe_hooks[i];

		if (!hook->registered)
			continue;
		unregister_kprobe(&hook->kp);
		hook->registered = false;
		pr_info(MODNAME ": redirect kprobe(%s) unregistered\n",
			hook->name);
	}
	socket_bind_hooks_registered = false;
}

static int register_socket_bind_hooks(void)
{
	int i, ret;

	WRITE_ONCE(socket_bind_hooks_ready, false);
	for (i = 0; i < ARRAY_SIZE(socket_bind_kprobe_hooks); i++) {
		struct socket_bind_kprobe_hook *hook =
			&socket_bind_kprobe_hooks[i];

		ret = register_kprobe(&hook->kp);
		if (ret)
			goto fail;
		hook->registered = true;
		if (i == 0)
			original_sock_setsockopt =
				(sock_setsockopt_fn)hook->kp.addr;
#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
		else
			original_sk_setsockopt =
				(sk_setsockopt_fn)hook->kp.addr;
#endif
		pr_info(MODNAME ": redirect kprobe(%s) registered\n",
			hook->name);
	}

	WRITE_ONCE(socket_bind_hooks_ready, true);
	socket_bind_hooks_registered = true;
	return 0;

fail:
	pr_warn(MODNAME ": redirect kprobe(%s) failed: %d\n",
		socket_bind_kprobe_hooks[i].name, ret);
	rollback_socket_bind_hooks();
	return ret;
}

/* ================================================================== */
/*  Optional filesystem path concealment (.ko only)                   */
/*                                                                    */
/*  These hooks sit on globally hot VFS paths, so they are installed  */
/*  only when filesystem_hiding=1 was selected before insmod. The     */
/*  target check remains per UID/per hook. Matching uses resolved     */
/*  dentries rather than userspace pathname strings, which also       */
/*  covers relative openat calls, symlink following, and bind mounts. */
/* ================================================================== */

struct open_flags;

typedef int (*filename_lookup_fn)(int dfd, struct filename *name,
				  unsigned int flags, struct path *path,
				  struct path *root);
typedef struct file *(*do_filp_open_fn)(int dfd, struct filename *name,
					const struct open_flags *op);
typedef int (*vfs_getattr_fn)(const struct path *path, struct kstat *stat,
			      u32 request_mask, unsigned int query_flags);
typedef int (*iterate_dir_fn)(struct file *file, struct dir_context *ctx);
typedef void (*path_put_fn)(const struct path *path);

static filename_lookup_fn original_filename_lookup;
static do_filp_open_fn original_do_filp_open;
static vfs_getattr_fn original_vfs_getattr;
static iterate_dir_fn original_iterate_dir;

/*
 * path_put() is exported in kallsyms but not on every OEM's GKI KMI module
 * symbol table — some OnePlus android14-6.1 kernels omit it — so a direct
 * call turns into an "Unknown symbol path_put" load failure that takes the
 * whole .ko down, including the core VPN-hiding hooks that never touch the
 * filesystem. Resolve it at runtime through a throwaway kprobe (kprobes walk
 * kallsyms, not the module export table) so the .ko links with no hard
 * reference, and call it only through call_resolved_path_put() below —
 * calling a kprobe-resolved address directly trips jump-table CFI on
 * android12/13-era kernels, exactly like the other VFS redirect originals.
 */
static path_put_fn resolved_path_put;

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

static bool filesystem_filter_active(void)
{
	return READ_ONCE(filesystem_hiding) &&
	       hook_active(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
}

static noinline __nocfi int call_original_filename_lookup(int dfd,
							  struct filename *name,
							  unsigned int flags,
							  struct path *path,
							  struct path *root)
{
	int ret = original_filename_lookup(dfd, name, flags, path, root);

	barrier();
	return ret;
}

/* __nocfi: resolved_path_put holds a kallsyms address recovered through a
 * kprobe, which is not a CFI jump-table entry — a checked indirect call to it
 * faults on android12/13-era jump-table CFI kernels, so bypass the check the
 * same way the VFS redirect originals do. */
static noinline __nocfi void call_resolved_path_put(const struct path *path)
{
	resolved_path_put(path);
	barrier();
}

static noinline int vpnhide_filename_lookup(int dfd, struct filename *name,
					    unsigned int flags,
					    struct path *path,
					    struct path *root)
{
	int ret = call_original_filename_lookup(dfd, name, flags, path, root);

	/* resolved_path_put may be NULL on kernels that neither export nor
	 * expose path_put; leave the resolved path intact rather than leak its
	 * reference — the interception is skipped, not the whole hook. */
	if (!ret && resolved_path_put && filesystem_filter_active() && path &&
	    dentry_is_hidden_iface_path(path->dentry)) {
		call_resolved_path_put(path);
		path->mnt = NULL;
		path->dentry = NULL;
		record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
		return -ENOENT;
	}
	return ret;
}

static noinline __nocfi struct file *
call_original_do_filp_open(int dfd, struct filename *name,
			   const struct open_flags *op)
{
	struct file *file = original_do_filp_open(dfd, name, op);

	barrier();
	return file;
}

static noinline struct file *vpnhide_do_filp_open(int dfd,
						  struct filename *name,
						  const struct open_flags *op)
{
	struct file *file = call_original_do_filp_open(dfd, name, op);

	if (!IS_ERR(file) && filesystem_filter_active() &&
	    dentry_is_hidden_iface_path(file->f_path.dentry)) {
		fput(file);
		record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
		return ERR_PTR(-ENOENT);
	}
	return file;
}

static noinline __nocfi int call_original_vfs_getattr(const struct path *path,
						      struct kstat *stat,
						      u32 request_mask,
						      unsigned int query_flags)
{
	int ret = original_vfs_getattr(path, stat, request_mask, query_flags);

	barrier();
	return ret;
}

static noinline int vpnhide_vfs_getattr(const struct path *path,
					struct kstat *stat, u32 request_mask,
					unsigned int query_flags)
{
	int ret = call_original_vfs_getattr(path, stat, request_mask,
					    query_flags);

	if (!ret && filesystem_filter_active() && path &&
	    dentry_is_hidden_iface_path(path->dentry)) {
		record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
		return -ENOENT;
	}
	return ret;
}

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
			record_hook_hit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
			return VPNHIDE_FILLDIR_CONTINUE;
		}
	}
	return actor ? actor(ctx, name, namelen, offset, ino, d_type) :
		       VPNHIDE_FILLDIR_CONTINUE;
}

static noinline __nocfi int call_original_iterate_dir(struct file *file,
						      struct dir_context *ctx)
{
	int ret = original_iterate_dir(file, ctx);

	barrier();
	return ret;
}

static noinline int vpnhide_iterate_dir(struct file *file,
					struct dir_context *ctx)
{
	struct readdir_filter_state *state;
	unsigned long irqflags;
	int ret;

	if (!filesystem_filter_active() || !file || !ctx ||
	    !dentry_is_iface_listing_dir(file->f_path.dentry))
		return call_original_iterate_dir(file, ctx);

	state = kmalloc(sizeof(*state), GFP_KERNEL);
	if (!state)
		return call_original_iterate_dir(file, ctx);
	state->ctx = ctx;
	state->original_actor = READ_ONCE(ctx->actor);
	spin_lock_irqsave(&readdir_filter_lock, irqflags);
	list_add(&state->node, &readdir_filter_states);
	spin_unlock_irqrestore(&readdir_filter_lock, irqflags);
	WRITE_ONCE(ctx->actor, vpnhide_filldir);

	ret = call_original_iterate_dir(file, ctx);

	WRITE_ONCE(ctx->actor, state->original_actor);
	spin_lock_irqsave(&readdir_filter_lock, irqflags);
	list_del(&state->node);
	spin_unlock_irqrestore(&readdir_filter_lock, irqflags);
	kfree(state);
	return ret;
}

struct filesystem_kprobe_hook {
	const char *name;
	unsigned long replacement;
	struct kprobe kp;
	bool registered;
};

static bool filesystem_hooks_ready;

static int filesystem_kprobe_pre(struct kprobe *kp, struct pt_regs *regs)
{
	struct filesystem_kprobe_hook *hook =
		container_of(kp, struct filesystem_kprobe_hook, kp);

	if (!READ_ONCE(filesystem_hooks_ready) ||
	    within_module(regs->regs[30], THIS_MODULE))
		return 0;
	instruction_pointer_set(regs, hook->replacement);
	return 1;
}

static void filesystem_kprobe_post(struct kprobe *kp, struct pt_regs *regs,
				   unsigned long flags)
{
	(void)kp;
	(void)regs;
	(void)flags;
}

static struct filesystem_kprobe_hook filesystem_kprobe_hooks[] = {
	{
		.name = "filename_lookup",
		.replacement = (unsigned long)vpnhide_filename_lookup,
		.kp = { .symbol_name = "filename_lookup",
			.pre_handler = filesystem_kprobe_pre,
			.post_handler = filesystem_kprobe_post },
	},
	{
		.name = "do_filp_open",
		.replacement = (unsigned long)vpnhide_do_filp_open,
		.kp = { .symbol_name = "do_filp_open",
			.pre_handler = filesystem_kprobe_pre,
			.post_handler = filesystem_kprobe_post },
	},
	{
		.name = "vfs_getattr",
		.replacement = (unsigned long)vpnhide_vfs_getattr,
		.kp = { .symbol_name = "vfs_getattr",
			.pre_handler = filesystem_kprobe_pre,
			.post_handler = filesystem_kprobe_post },
	},
	{
		.name = "iterate_dir",
		.replacement = (unsigned long)vpnhide_iterate_dir,
		.kp = { .symbol_name = "iterate_dir",
			.pre_handler = filesystem_kprobe_pre,
			.post_handler = filesystem_kprobe_post },
	},
};

static bool filesystem_hooks_registered;

static void rollback_filesystem_hooks(void)
{
	int i;

	WRITE_ONCE(filesystem_hooks_ready, false);
	for (i = ARRAY_SIZE(filesystem_kprobe_hooks) - 1; i >= 0; i--) {
		if (!filesystem_kprobe_hooks[i].registered)
			continue;
		unregister_kprobe(&filesystem_kprobe_hooks[i].kp);
		filesystem_kprobe_hooks[i].registered = false;
	}
	filesystem_hooks_registered = false;
}

/*
 * Recover a kernel symbol's address through a throwaway kprobe — the same
 * addr-grab the VFS redirect hooks use for their non-exported originals.
 * kprobes resolve through kallsyms, so this reaches symbols the running
 * kernel does not export to modules (e.g. path_put on some OnePlus KMIs).
 */
static void *resolve_kernel_symbol(const char *name)
{
	struct kprobe kp = { .symbol_name = name };
	void *addr = NULL;

	if (register_kprobe(&kp) == 0) {
		addr = kp.addr;
		unregister_kprobe(&kp);
	}
	return addr;
}

static int register_filesystem_hooks(void)
{
	int i, ret;

	/* Best-effort: if path_put cannot be resolved, the other filesystem
	 * hooks still register and vpnhide_filename_lookup simply skips its one
	 * interception (guarded on resolved_path_put). */
	if (!resolved_path_put)
		resolved_path_put =
			(path_put_fn)resolve_kernel_symbol("path_put");

	for (i = 0; i < ARRAY_SIZE(filesystem_kprobe_hooks); i++) {
		struct filesystem_kprobe_hook *hook =
			&filesystem_kprobe_hooks[i];

		ret = register_kprobe(&hook->kp);
		if (ret)
			goto fail;
		hook->registered = true;
		switch (i) {
		case 0:
			original_filename_lookup =
				(filename_lookup_fn)hook->kp.addr;
			break;
		case 1:
			original_do_filp_open = (do_filp_open_fn)hook->kp.addr;
			break;
		case 2:
			original_vfs_getattr = (vfs_getattr_fn)hook->kp.addr;
			break;
		case 3:
			original_iterate_dir = (iterate_dir_fn)hook->kp.addr;
			break;
		}
		pr_info(MODNAME ": VFS redirect kprobe(%s) registered\n",
			hook->name);
	}
	WRITE_ONCE(filesystem_hooks_ready, true);
	filesystem_hooks_registered = true;
	return 0;

fail:
	pr_warn(MODNAME ": VFS redirect kprobe(%s) failed: %d\n",
		filesystem_kprobe_hooks[i].name, ret);
	rollback_filesystem_hooks();
	return ret;
}

/* ================================================================== */
/*  Hook 2: sock_ioctl — SIOCGIFCONF interface enumeration            */
/*                                                                    */
/*  Why sock_ioctl instead of dev_ifconf?                             */
/*                                                                    */
/*  On GKI 5.10 kernels built with Clang LTO, the linker can inline   */
/*  dev_ifconf() into sock_do_ioctl().                                */
/*  The symbol "dev_ifconf" stays in kallsyms as a dead stub, so      */
/*  kretprobe registration succeeds but the probe never fires.        */
/*  The unused dev_ifconf symbol then has no live call from           */
/*  sock_do_ioctl, despite successful kretprobe registration.         */
/*                                                                    */
/*  On 6.1+, SIOCGIFCONF was moved out of sock_do_ioctl() into       */
/*  sock_ioctl() directly (handled in the switch statement), so       */
/*  hooking sock_do_ioctl would miss it on newer kernels.             */
/*                                                                    */
/*  sock_ioctl is the correct hook point because:                     */
/*  1. It is the file_operations->unlocked_ioctl callback for socket  */
/*     fds — used as a function pointer, so LTO cannot inline it.     */
/*  2. Supported GKI paths dispatch socket ioctls through it.         */
/*  3. After sock_ioctl returns, the ifconf data (ifreq array +       */
/*     ifc_len) is already in userspace — we filter it uniformly via  */
/*     copy_from_user/copy_to_user regardless of kernel version.      */
/*                                                                    */
/*  sock_ioctl(struct file *file, unsigned int cmd, unsigned long arg) */
/*  arm64: x0=file, x1=cmd, x2=arg (__user ptr)                      */
/*                                                                    */
/*  Performance: entry handler checks cmd == SIOCGIFCONF first (one   */
/*  compare), then hook_active(). For all other ioctls, overhead      */
/*  is a single branch. SIOCGIFCONF is rare (once per getifaddrs).    */
/* ================================================================== */

struct sock_ioctl_data {
	void __user *argp;
	/* Net namespace of the socket this ioctl is on — captured at entry so
	 * the size-query path (ifc_req == NULL) can enumerate that ns's netdevs
	 * to learn how many ifreqs the kernel counted for VPN ifaces. dev_ifconf
	 * uses sock_net(sk), so we match it instead of guessing current's ns. */
	struct net *net;
	bool target;
};

static int sock_ioctl_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct sock_ioctl_data *data = (void *)ri->data;
	unsigned int cmd = (unsigned int)regs->regs[1];
	struct file *file;
	struct socket *sock;

	data->target = false;

	if (cmd != SIOCGIFCONF)
		return 0;
	if (!hook_active(VPNHIDE_HOOK_SOCK_IOCTL))
		return 0;

	/* sock_ioctl is the socket file_operations callback, so private_data is
	 * the struct socket, matching sock_from_file()'s internal invariant. */
	file = (struct file *)regs->regs[0];
	sock = file ? file->private_data : NULL;
	data->net = (sock && sock->sk) ? sock_net(sock->sk) : NULL;

	data->target = true;
	data->argp = (void __user *)regs->regs[2];
	vpnhide_dbg("sock_ioctl_entry: uid=%u SIOCGIFCONF argp=%px\n",
		    from_kuid(&init_user_ns, current_uid()), data->argp);
	return 0;
}

/*
 * Why user-memory access is OK here:
 *
 * `sock_ioctl_ret` runs as a kretprobe return handler — same process
 * context that issued the SIOCGIFCONF syscall, kernel mode, original
 * task is still mapped and addressable. copy_from_user/copy_to_user
 * are safe in this context (it's the same userspace the original
 * sock_ioctl handler accessed). PAN/uaccess primitives are honoured.
 *
 * If the caller races by unmapping its buffer, uaccess reports a fault and
 * filtering stops. The buffer may already be partly rewritten, so the caller
 * keeps the original ifc_len rather than receiving a shortened length that
 * claims the incomplete transformation succeeded.
 */
enum filter_ifconf_result {
	FILTER_IFCONF_NO_CHANGE,
	FILTER_IFCONF_CHANGED,
	FILTER_IFCONF_COPY_FAULT,
};

/* Compact VPN entries out of the userspace ifreq array and clear the slots
 * removed from the kernel-written range. The caller is responsible for
 * updating `ifc_len` only on FILTER_IFCONF_CHANGED. */
static enum filter_ifconf_result filter_ifconf_buf(struct ifreq __user *usr_ifr,
						   int n, int *out_len)
{
	struct ifreq tmp;
	int i, dst = 0;

	for (i = 0; i < n; i++) {
		if (copy_from_user(&tmp, &usr_ifr[i], sizeof(tmp)))
			return FILTER_IFCONF_COPY_FAULT;
		tmp.ifr_name[IFNAMSIZ - 1] = '\0';
		if (is_vpn_ifname(tmp.ifr_name))
			continue;
		if (dst != i) {
			if (copy_to_user(&usr_ifr[dst], &tmp, sizeof(tmp)))
				return FILTER_IFCONF_COPY_FAULT;
		}
		dst++;
	}

	if (dst == n)
		return FILTER_IFCONF_NO_CHANGE;

	/* Shortening ifc_len alone leaves the removed kernel output readable in
	 * the caller-owned tail. Clear only slots the kernel returned, never the
	 * unused remainder of the caller's buffer. */
	if (clear_user(&usr_ifr[dst], (n - dst) * sizeof(struct ifreq)))
		return FILTER_IFCONF_COPY_FAULT;

	*out_len = dst * (int)sizeof(struct ifreq);
	return FILTER_IFCONF_CHANGED;
}

/*
 * Size-query subcase: SIOCGIFCONF with ifc_req == NULL. The kernel
 * (dev_ifconf -> inet_gifconf) doesn't copy any ifreqs, it just returns
 * ifc_len = (number of IPv4 addresses across all netdevs) * sizeof(ifreq).
 * There's no buffer to compact, so to keep the size query consistent with the
 * filtered fill, we recompute how many of those ifreqs belong to VPN ifaces
 * and shrink ifc_len by that much. Otherwise a target doing the classic
 * two-step probe (size, then fill) sees the fill come back one interface short
 * of the advertised size.
 *
 * inet_gifconf emits one ifreq per in_ifaddr, named by ifa_label, so we count
 * by ifa_label under rcu — the exact set and naming the fill path would filter.
 */
static void filter_ifconf_size_probe(struct net *net,
				     struct ifconf __user *uifc, int orig_len)
{
	struct net_device *dev;
	int hidden = 0;
	int new_len;

	if (!net)
		return;

	rcu_read_lock();
	for_each_netdev_rcu(net, dev) {
		struct in_device *in_dev = __in_dev_get_rcu(dev);
		const struct in_ifaddr *ifa;

		if (!in_dev)
			continue;
		in_dev_for_each_ifa_rcu(ifa, in_dev) {
			char label[IFNAMSIZ];

			memcpy(label, ifa->ifa_label, IFNAMSIZ);
			label[IFNAMSIZ - 1] = '\0';
			if (is_vpn_ifname(label))
				hidden++;
		}
	}
	rcu_read_unlock();

	if (hidden == 0)
		return;

	new_len = orig_len - hidden * (int)sizeof(struct ifreq);
	if (new_len < 0)
		new_len = 0;
	if (put_user(new_len, &uifc->ifc_len)) {
		vpnhide_dbg(
			"ifconf size-probe: put_user failed; len untouched\n");
		return;
	}
	vpnhide_dbg("ifconf size-probe %d -> %d (hid %d vpn addr)\n", orig_len,
		    new_len, hidden);
	record_hook_hit(VPNHIDE_HOOK_SOCK_IOCTL);
}

static int sock_ioctl_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct sock_ioctl_data *data = (void *)ri->data;
	struct ifconf __user *uifc;
	struct ifconf ifc;
	int orig_len;
	enum filter_ifconf_result res;

	if (!data->target)
		return 0;

	vpnhide_dbg("sock_ioctl_ret: retval=%ld argp=%px\n",
		    regs_return_value(regs), data->argp);

	if (regs_return_value(regs) != 0 || !data->argp)
		return 0;

	uifc = data->argp;
	if (copy_from_user(&ifc, uifc, sizeof(ifc)))
		return 0;
	if (ifc.ifc_len <= 0)
		return 0;

	/* ifc_req == NULL is the size-query probe (no buffer to compact). */
	if (!ifc.ifc_req) {
		filter_ifconf_size_probe(data->net, uifc, ifc.ifc_len);
		return 0;
	}

	orig_len = ifc.ifc_len;
	res = filter_ifconf_buf(ifc.ifc_req,
				ifc.ifc_len / (int)sizeof(struct ifreq),
				&ifc.ifc_len);

	if (res == FILTER_IFCONF_COPY_FAULT) {
		/*
		 * Partial copy failure — buffer may already be
		 * half-rewritten. Don't update ifc_len: a shorter
		 * length on a partially-compacted buffer hides VPN
		 * entries past the truncation but lets earlier ones
		 * through, which is worse than just leaving
		 * everything visible. Userspace sees the original
		 * length and the (mostly-original) buffer.
		 */
		vpnhide_dbg(
			"ifconf: copy fault during filter; ifc_len untouched\n");
		return 0;
	}

	if (res == FILTER_IFCONF_CHANGED) {
		if (put_user(ifc.ifc_len, &uifc->ifc_len)) {
			vpnhide_dbg(
				"ifconf: put_user(ifc_len=%d) failed; userspace will see compacted buffer with stale length\n",
				ifc.ifc_len);
			return 0;
		}
		vpnhide_dbg("ifconf filtered %d -> %d bytes\n", orig_len,
			    ifc.ifc_len);
		record_hook_hit(VPNHIDE_HOOK_SOCK_IOCTL);
	}

	return 0;
}

static struct kretprobe sock_ioctl_krp = {
	.handler = sock_ioctl_ret,
	.entry_handler = sock_ioctl_entry,
	.data_size = sizeof(struct sock_ioctl_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "sock_ioctl",
};

/* ================================================================== */
/*  Hook 3: rtnl_fill_ifinfo — netlink RTM_NEWLINK (getifaddrs path)  */
/*                                                                    */
/*  rtnl_fill_ifinfo fills one interface's data into a netlink skb    */
/*  during a RTM_GETLINK dump. If the device is a VPN and the caller  */
/*  is a target UID, we hide the entry from the dump.                 */
/*                                                                    */
/*  We can't return -EMSGSIZE (causes infinite retry of the same      */
/*  entry on android14-6.1, hanging RTM_GETLINK dumps). Instead use   */
/*  the same skb_trim approach as inet6_fill_ifaddr below: save       */
/*  skb->len before the fill, trim back on return, return 0. The      */
/*  iterator then sees a successful entry of zero bytes and advances. */
/* ================================================================== */

struct rtnl_fill_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static int rtnl_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rtnl_fill_data *data = (void *)ri->data;
	struct net_device *dev;

	data->should_filter = false;

	if (!hook_active(VPNHIDE_HOOK_RTNL_FILL_IFINFO)) {
		vpnhide_dbg("rtnl_fill_entry: uid=%u target=0\n",
			    from_kuid(&init_user_ns, current_uid()));
		return 0;
	}

	/*
	 * rtnl_fill_ifinfo(struct sk_buff *skb, struct net_device *dev, ...)
	 * arm64: x0=skb, x1=dev
	 */
	dev = (struct net_device *)regs->regs[1];
	/* Callers hold RTNL which protects dev->name, but take RCU as
	 * belt-and-suspenders — same rationale as inet6_fill_entry. */
	rcu_read_lock();
	if (dev && is_vpn_ifname(dev->name)) {
		data->skb = (struct sk_buff *)regs->regs[0];
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg(
			"rtnl_fill_entry: uid=%u target=1 iface=%s -> filter\n",
			from_kuid(&init_user_ns, current_uid()), dev->name);
	} else {
		vpnhide_dbg(
			"rtnl_fill_entry: uid=%u target=1 iface=%s -> pass\n",
			from_kuid(&init_user_ns, current_uid()),
			dev ? dev->name : "(null)");
	}
	rcu_read_unlock();

	return 0;
}

static int rtnl_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rtnl_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	vpnhide_dbg("rtnl_fill_ret: trimming skb %u -> %u\n", data->skb->len,
		    data->saved_len);
	/* Undo whatever the fill function wrote to the skb */
	skb_trim(data->skb, data->saved_len);
	regs_set_return_value(regs, 0);
	record_hook_hit(VPNHIDE_HOOK_RTNL_FILL_IFINFO);
	return 0;
}

static struct kretprobe rtnl_fill_krp = {
	.handler = rtnl_fill_ret,
	.entry_handler = rtnl_fill_entry,
	.data_size = sizeof(struct rtnl_fill_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "rtnl_fill_ifinfo",
};

/* ================================================================== */
/*  Hook 4: inet6_fill_ifaddr — RTM_GETADDR IPv6 (getifaddrs path)   */
/*                                                                    */
/*  inet6_fill_ifaddr(struct sk_buff *skb, struct inet6_ifaddr *ifa,  */
/*                    struct inet6_fill_args *args)                   */
/*  arm64: x0=skb, x1=ifa                                           */
/*                                                                    */
/*  getifaddrs() does RTM_GETLINK (filtered by hook 3) then          */
/*  RTM_GETADDR. Addresses for VPN interfaces still appear in        */
/*  RTM_GETADDR, so bionic reconstructs a tun0 entry with flags=0.  */
/*  Filtering here prevents that.                                    */
/*                                                                    */
/*  We can't return -EMSGSIZE (causes infinite retry on empty skb).  */
/*  Instead, save skb->len before and trim the skb back on return,   */
/*  making it look like the entry was never written. Return 0.       */
/* ================================================================== */

struct inet6_fill_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static int inet6_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct inet6_fill_data *data = (void *)ri->data;
	struct inet6_ifaddr *ifa;

	data->should_filter = false;

	if (!hook_active(VPNHIDE_HOOK_INET6_FILL_IFADDR))
		return 0;

	ifa = (struct inet6_ifaddr *)regs->regs[1];
	/*
	 * The callers of inet6_fill_ifaddr() hold either rcu_read_lock()
	 * (netlink dump path) or RTNL. We take rcu_read_lock() explicitly
	 * so the kretprobe handler doesn't rely on that implicit guarantee.
	 */
	rcu_read_lock();
	if (ifa && ifa->idev && ifa->idev->dev &&
	    is_vpn_ifname(ifa->idev->dev->name)) {
		data->skb = (struct sk_buff *)regs->regs[0];
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg("inet6_fill_entry: uid=%u iface=%s -> filter\n",
			    from_kuid(&init_user_ns, current_uid()),
			    ifa->idev->dev->name);
	}
	rcu_read_unlock();

	return 0;
}

static int inet6_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct inet6_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	vpnhide_dbg("inet6_fill_ret: trimming skb %u -> %u\n", data->skb->len,
		    data->saved_len);
	/* Undo whatever the fill function wrote to the skb */
	skb_trim(data->skb, data->saved_len);
	regs_set_return_value(regs, 0);
	record_hook_hit(VPNHIDE_HOOK_INET6_FILL_IFADDR);
	return 0;
}

static struct kretprobe inet6_fill_krp = {
	.handler = inet6_fill_ret,
	.entry_handler = inet6_fill_entry,
	.data_size = sizeof(struct inet6_fill_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "inet6_fill_ifaddr",
};

/* ================================================================== */
/*  Hook 5: inet_fill_ifaddr — RTM_GETADDR IPv4 (getifaddrs path)    */
/*                                                                    */
/*  inet_fill_ifaddr(struct sk_buff *skb, struct in_ifaddr *ifa,     */
/*                   struct inet_fill_args *args)                    */
/*  arm64: x0=skb, x1=ifa                                           */
/*  Same skb-trim approach as hook 4.                                */
/* ================================================================== */

struct inet_fill_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static int inet_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct inet_fill_data *data = (void *)ri->data;
	struct in_ifaddr *ifa;

	data->should_filter = false;

	if (!hook_active(VPNHIDE_HOOK_INET_FILL_IFADDR))
		return 0;

	ifa = (struct in_ifaddr *)regs->regs[1];
	/* Same RCU rationale as inet6_fill_entry above. */
	rcu_read_lock();
	if (ifa && ifa->ifa_dev && ifa->ifa_dev->dev &&
	    is_vpn_ifname(ifa->ifa_dev->dev->name)) {
		data->skb = (struct sk_buff *)regs->regs[0];
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg("inet_fill_entry: uid=%u iface=%s -> filter\n",
			    from_kuid(&init_user_ns, current_uid()),
			    ifa->ifa_dev->dev->name);
	}
	rcu_read_unlock();

	return 0;
}

static int inet_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct inet_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	vpnhide_dbg("inet_fill_ret: trimming skb %u -> %u\n", data->skb->len,
		    data->saved_len);
	skb_trim(data->skb, data->saved_len);
	regs_set_return_value(regs, 0);
	record_hook_hit(VPNHIDE_HOOK_INET_FILL_IFADDR);
	return 0;
}

static struct kretprobe inet_fill_krp = {
	.handler = inet_fill_ret,
	.entry_handler = inet_fill_entry,
	.data_size = sizeof(struct inet_fill_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "inet_fill_ifaddr",
};

/* ================================================================== */
/*  Hook 6: fib_route_seq_show — /proc/net/route                      */
/*                                                                    */
/*  fib_route_seq_show(struct seq_file *seq, void *v) writes one or  */
/*  more tab-separated route lines into seq->buf, each ending with   */
/*  '\n'. The first field is the interface name.                      */
/*                                                                    */
/*  We save seq and seq->count on entry. In the return handler we    */
/*  scan what was written, compact out VPN lines, and adjust count.  */
/* ================================================================== */

struct fib_route_data {
	struct seq_file *seq;
	size_t start_count;
	bool target;
};

static int fib_route_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct fib_route_data *data = (void *)ri->data;

	/*
	 * arm64: x0 = seq_file*, x1 = v (iterator element).
	 * Save seq pointer and current buffer position so the
	 * return handler knows where this call's output begins.
	 */
	data->seq = (struct seq_file *)regs->regs[0];
	data->target = hook_active(VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW);

	if (data->target && data->seq) {
		data->start_count = data->seq->count;
		vpnhide_dbg("fib_route_entry: uid=%u target=1\n",
			    from_kuid(&init_user_ns, current_uid()));
	} else {
		data->start_count = 0;
	}

	return 0;
}

/*
 * We access seq->buf and seq->count without seq_file's internal mutex.
 * This is safe because seq_read() drives the ->show() callback
 * synchronously under its own fd context — no concurrent access to
 * the same seq_file is possible between our entry and return handlers.
 */
static int fib_route_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct fib_route_data *data = (void *)ri->data;
	struct seq_file *seq = data->seq;
	unsigned long newc;

	if (!data->target || !seq || !seq->buf)
		return 0;
	if (seq->count <= data->start_count)
		return 0;

	/*
	 * Compact out lines whose FIRST tab-separated field (each route line is
	 * "tun0\t08000000\t...\n") is a VPN iface name, in [start_count, count).
	 * Uses the shared compactor — the single implementation the KPM also
	 * calls — instead of an open-coded copy.
	 */
	newc = vpnhide_compact_seq_lines(seq->buf, data->start_count,
					 seq->count, VPNHIDE_FIELD_FIRST,
					 vpnhide_iface_is_vpn);
	if (newc != seq->count) {
		seq->count = newc;
		record_hook_hit(VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW);
	}
	return 0;
}

static struct kretprobe fib_route_krp = {
	.handler = fib_route_ret,
	.entry_handler = fib_route_entry,
	.data_size = sizeof(struct fib_route_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "fib_route_seq_show",
};

/* ================================================================== */
/*  Hook 7: ipv6_route_seq_show — /proc/net/ipv6_route                */
/*                                                                    */
/*  IPv6 route lines store the interface name in the final field.     */
/*  We compact VPN lines out of the seq_file buffer, matching the     */
/*  IPv4 /proc/net/route strategy above.                              */
/* ================================================================== */

static int ipv6_route_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct fib_route_data *data = (void *)ri->data;

	data->seq = (struct seq_file *)regs->regs[0];
	data->target = hook_active(VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW);

	if (data->target && data->seq) {
		data->start_count = data->seq->count;
		vpnhide_dbg("ipv6_route_entry: uid=%u target=1\n",
			    from_kuid(&init_user_ns, current_uid()));
	} else {
		data->start_count = 0;
	}

	return 0;
}

static int ipv6_route_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct fib_route_data *data = (void *)ri->data;
	struct seq_file *seq = data->seq;
	unsigned long newc;

	if (!data->target || !seq || !seq->buf)
		return 0;
	if (seq->count <= data->start_count)
		return 0;

	/* Same as fib_route_ret but the iface name is the LAST whitespace field
	 * of each /proc/net/ipv6_route line — the shared compactor handles both
	 * via the field selector. */
	newc = vpnhide_compact_seq_lines(seq->buf, data->start_count,
					 seq->count, VPNHIDE_FIELD_LAST,
					 vpnhide_iface_is_vpn);
	if (newc != seq->count) {
		seq->count = newc;
		record_hook_hit(VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW);
	}
	return 0;
}

static struct kretprobe ipv6_route_krp = {
	.handler = ipv6_route_ret,
	.entry_handler = ipv6_route_entry,
	.data_size = sizeof(struct fib_route_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "ipv6_route_seq_show",
};

/* ================================================================== */
/*  Route netlink helpers                                             */
/* ================================================================== */

static bool copy_dev_name(struct net_device *dev, char name[IFNAMSIZ])
{
	if (!dev)
		return false;
	if (copy_from_kernel_nofault(name, dev->name, IFNAMSIZ) != 0)
		return false;
	name[IFNAMSIZ - 1] = '\0';
	return true;
}

/* A public /32 host-route pinned to a physical uplink — the route a VPN client
 * installs so tunnel packets reach the server, leaking the server's IPv4 even
 * when the tun iface is hidden. The address/iface logic is shared with the KPM
 * (vpnhide_is_public_ipv4 / vpnhide_iface_is_physical in shared/vpnhide_logic.h);
 * &fri->dst is the __be32's 4 network-order bytes. */
static bool is_public_host_route_via_physical(const struct fib_rt_info *fri,
					      struct net_device *dev)
{
	char name[IFNAMSIZ];

	if (!fri || !dev || fri->dst_len != 32 ||
	    !vpnhide_is_public_ipv4((const unsigned char *)&fri->dst))
		return false;
	if (!copy_dev_name(dev, name))
		return false;
	return vpnhide_iface_is_physical(name);
}

/* IPv6 analogue of is_public_host_route_via_physical: a /128 route to a
 * public address pinned to a physical interface is the host-route a VPN
 * client installs so tunnel packets can reach the server — it leaks the
 * server's IPv6 even when the tun interface itself is hidden. fib6_dst
 * (struct rt6key { struct in6_addr addr; int plen; }) is stable across
 * GKI 5.10..6.12; read it fault-safe since `rt` comes from a raw reg. The
 * address/iface logic is shared with the KPM (shared/vpnhide_logic.h). */
static bool is_public_host_route6_via_physical(struct fib6_info *rt,
					       struct net_device *dev)
{
	struct in6_addr addr;
	int plen = 0;
	char name[IFNAMSIZ];

	if (!rt || !dev)
		return false;
	if (copy_from_kernel_nofault(&plen, &rt->fib6_dst.plen, sizeof(plen)) !=
		    0 ||
	    plen != 128)
		return false;
	if (copy_from_kernel_nofault(&addr, &rt->fib6_dst.addr, sizeof(addr)) !=
		    0 ||
	    !vpnhide_is_public_ipv6(addr.s6_addr))
		return false;
	if (!copy_dev_name(dev, name))
		return false;
	return vpnhide_iface_is_physical(name);
}

static struct net_device *dev_from_nexthop(struct nexthop *nh)
{
	struct net_device *dev = NULL;
	bool is_group = false;

	if (!nh)
		return NULL;

	if (copy_from_kernel_nofault(&is_group, &nh->is_group,
				     sizeof(is_group)) != 0)
		return NULL;

	if (is_group) {
		struct nh_group *nh_grp = NULL;
		struct nexthop *first_nh = NULL;
		u16 num_nh = 0;

		if (copy_from_kernel_nofault(&nh_grp, &nh->nh_grp,
					     sizeof(nh_grp)) != 0 ||
		    !nh_grp)
			return NULL;
		if (copy_from_kernel_nofault(&num_nh, &nh_grp->num_nh,
					     sizeof(num_nh)) != 0 ||
		    num_nh == 0)
			return NULL;
		if (copy_from_kernel_nofault(&first_nh,
					     &nh_grp->nh_entries[0].nh,
					     sizeof(first_nh)) != 0 ||
		    !first_nh)
			return NULL;
		nh = first_nh;
	}

	{
		struct nh_info *nhi = NULL;

		if (copy_from_kernel_nofault(&nhi, &nh->nh_info, sizeof(nhi)) ==
			    0 &&
		    nhi) {
			copy_from_kernel_nofault(&dev, &nhi->fib_nhc.nhc_dev,
						 sizeof(dev));
		}
	}

	return dev;
}

static struct net_device *dev_from_fib_info(struct fib_info *fi)
{
	struct net_device *dev = NULL;
	struct nexthop *nh = NULL;

	if (!fi)
		return NULL;

	if (copy_from_kernel_nofault(&nh, &fi->nh, sizeof(nh)) == 0 && nh) {
		dev = dev_from_nexthop(nh);
	} else {
		int fib_nhs = 0;

		if (copy_from_kernel_nofault(&fib_nhs, &fi->fib_nhs,
					     sizeof(fib_nhs)) == 0 &&
		    fib_nhs > 0) {
			copy_from_kernel_nofault(
				&dev, &fi->fib_nh[0].nh_common.nhc_dev,
				sizeof(dev));
		}
	}

	return dev;
}

static struct net_device *dev_from_fib6_info(struct fib6_info *rt)
{
	struct net_device *dev = NULL;
	struct nexthop *nh = NULL;

	if (!rt)
		return NULL;

	if (copy_from_kernel_nofault(&nh, &rt->nh, sizeof(nh)) == 0 && nh) {
		dev = dev_from_nexthop(nh);
	} else {
		copy_from_kernel_nofault(
			&dev, &rt->fib6_nh[0].nh_common.nhc_dev, sizeof(dev));
	}

	return dev;
}

struct route_skb_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static void init_route_skb_data(struct route_skb_data *data)
{
	data->skb = NULL;
	data->saved_len = 0;
	data->should_filter = false;
}

static int route_skb_ret(struct route_skb_data *data, struct pt_regs *regs,
			 const char *hook_name, enum vpnhide_hook_id hook_id)
{
	if (!data->should_filter || !data->skb)
		return 0;

	if (regs_return_value(regs) >= 0) {
		vpnhide_dbg("%s: trimming skb %u -> %u\n", hook_name,
			    data->skb->len, data->saved_len);
		skb_trim(data->skb, data->saved_len);
		regs_set_return_value(regs, 0);
		record_hook_hit(hook_id);
	}
	return 0;
}

/* ================================================================== */
/*  Hook 8: fib_dump_info — IPv4 RTM_GETROUTE dumps                   */
/*                                                                    */
/*  arm64: x0=skb, x4=fri (struct fib_rt_info*)                      */
/* ================================================================== */

static int fib_dump_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct route_skb_data *data = (void *)ri->data;
	struct fib_rt_info *fri = (struct fib_rt_info *)regs->regs[4];
	struct fib_rt_info fri_copy;
	struct net_device *dev = NULL;
	char dev_name[IFNAMSIZ];
	bool vpn_route;
	bool host_hint;

	init_route_skb_data(data);

	if (!hook_active(VPNHIDE_HOOK_FIB_DUMP_INFO) || !fri)
		return 0;
	if (copy_from_kernel_nofault(&fri_copy, fri, sizeof(fri_copy)) != 0)
		return 0;

	rcu_read_lock();
	dev = dev_from_fib_info(fri_copy.fi);
	if (!copy_dev_name(dev, dev_name)) {
		rcu_read_unlock();
		return 0;
	}
	vpn_route = is_vpn_ifname(dev_name);
	host_hint = is_public_host_route_via_physical(&fri_copy, dev);
	if (vpn_route || host_hint) {
		data->skb = (struct sk_buff *)regs->regs[0];
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg("fib_dump_entry: hiding %s via %s\n",
			    vpn_route ? "VPN route" : "public host route",
			    dev_name);
	}
	rcu_read_unlock();

	return 0;
}

static int fib_dump_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	return route_skb_ret((void *)ri->data, regs, "fib_dump_ret",
			     VPNHIDE_HOOK_FIB_DUMP_INFO);
}

static struct kretprobe fib_dump_krp = {
	.handler = fib_dump_ret,
	.entry_handler = fib_dump_entry,
	.data_size = sizeof(struct route_skb_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "fib_dump_info",
};

/* ================================================================== */
/*  Hook 9: rt6_fill_node — IPv6 RTM_GETROUTE                         */
/*                                                                    */
/*  arm64: x1=skb, x2=rt (struct fib6_info*), x3=dst                 */
/* ================================================================== */

static int rt6_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct route_skb_data *data = (void *)ri->data;
	struct fib6_info *rt = (struct fib6_info *)regs->regs[2];
	struct dst_entry *dst = (struct dst_entry *)regs->regs[3];
	struct net_device *dev = NULL;
	char dev_name[IFNAMSIZ];

	init_route_skb_data(data);

	if (!hook_active(VPNHIDE_HOOK_RT6_FILL_NODE))
		return 0;

	rcu_read_lock();
	dev = dev_from_fib6_info(rt);
	if (!dev && dst)
		copy_from_kernel_nofault(&dev, &dst->dev, sizeof(dev));
	if (copy_dev_name(dev, dev_name)) {
		bool vpn_route = is_vpn_ifname(dev_name);
		bool host_hint = !vpn_route &&
				 is_public_host_route6_via_physical(rt, dev);

		if (vpn_route || host_hint) {
			data->skb = (struct sk_buff *)regs->regs[1];
			data->saved_len = data->skb ? data->skb->len : 0;
			data->should_filter = true;
			vpnhide_dbg("rt6_fill_entry: hiding %s via %s\n",
				    vpn_route ? "VPN route" :
						"public host route",
				    dev_name);
		}
	}
	rcu_read_unlock();

	return 0;
}

static int rt6_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	return route_skb_ret((void *)ri->data, regs, "rt6_fill_ret",
			     VPNHIDE_HOOK_RT6_FILL_NODE);
}

static struct kretprobe rt6_fill_krp = {
	.handler = rt6_fill_ret,
	.entry_handler = rt6_fill_entry,
	.data_size = sizeof(struct route_skb_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "rt6_fill_node",
};

/*
 * Note: rt_fill_info (single-lookup RTM_GETROUTE serializer for
 * `ip route get <dst>`) is intentionally NOT hooked.
 *
 * It is a `static` function called directly within net/ipv4/route.c, so
 * the compiler is free to ignore AAPCS64 and assign its arguments to
 * arbitrary registers (interprocedural register allocation). Verified in
 * QEMU on a no-LTO android12-5.10 build: regs[3] held table_id (254),
 * not the `struct rtable *` the source signature places there — so no
 * fixed regs[N] read is correct across builds (the value differs between
 * LTO device builds and no-LTO builds). A hardcoded register is build-
 * dependent guesswork that fails silently (or panics, without nofault).
 *
 * It is also low value here: IPv4 route *enumeration* (RTM_GETROUTE with
 * NLM_F_DUMP — what detection apps actually use) goes through the global,
 * ABI-stable fib_dump_info hook above, not rt_fill_info. Single lookups
 * respect the caller's own routing, which under the recommended split-
 * tunnel setup resolves to the physical interface anyway.
 *
 * If single-lookup concealment is ever needed, hook rtnl_unicast instead
 * (global EXPORT_SYMBOL, ABI-stable, runs in caller context) and rewrite
 * RTA_OIF in the reply skb — see docs/ROADMAP.md.
 */

/* ================================================================== */
/*  Hook 10: fib_nl_fill_rule — RTM_GETRULE policy rules              */
/*                                                                    */
/*  arm64: x0=skb, x1=rule (struct fib_rule*)                        */
/* ================================================================== */

static int fib_rule_fill_entry(struct kretprobe_instance *ri,
			       struct pt_regs *regs)
{
	struct route_skb_data *data = (void *)ri->data;
	struct fib_rule *rule = (struct fib_rule *)regs->regs[1];
	struct fib_rule rule_copy;
	uid_t uid;
	bool filter = false;

	init_route_skb_data(data);

	if (!hook_active(VPNHIDE_HOOK_FIB_NL_FILL_RULE) || !rule)
		return 0;
	if (copy_from_kernel_nofault(&rule_copy, rule, sizeof(rule_copy)) != 0)
		return 0;

	uid = from_kuid(&init_user_ns, current_uid());

	if ((rule_copy.iifname[0] != '\0' &&
	     is_vpn_ifname(rule_copy.iifname)) ||
	    (rule_copy.oifname[0] != '\0' &&
	     is_vpn_ifname(rule_copy.oifname))) {
		filter = true;
	} else {
		uid_t start =
			from_kuid(&init_user_ns, rule_copy.uid_range.start);
		uid_t end = from_kuid(&init_user_ns, rule_copy.uid_range.end);

		if (uid >= start && uid <= end &&
		    (start != 0 || end != (uid_t)~0) &&
		    rule_copy.table != RT_TABLE_MAIN &&
		    rule_copy.table != RT_TABLE_LOCAL &&
		    rule_copy.table != RT_TABLE_DEFAULT &&
		    rule_copy.table > 100) {
			filter = true;
		}
	}

	if (filter) {
		data->skb = (struct sk_buff *)regs->regs[0];
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg(
			"fib_rule_fill_entry: hiding policy rule table=%u\n",
			rule_copy.table);
	}

	return 0;
}

static int fib_rule_fill_ret(struct kretprobe_instance *ri,
			     struct pt_regs *regs)
{
	return route_skb_ret((void *)ri->data, regs, "fib_rule_fill_ret",
			     VPNHIDE_HOOK_FIB_NL_FILL_RULE);
}

static struct kretprobe fib_rule_fill_krp = {
	.handler = fib_rule_fill_ret,
	.entry_handler = fib_rule_fill_entry,
	.data_size = sizeof(struct route_skb_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "fib_nl_fill_rule",
};

/* ================================================================== */
/*  Module init                                                       */
/* ================================================================== */

struct kretprobe_reg {
	struct kretprobe *krp;
	enum vpnhide_hook_id hook_id;
	bool registered;
};

static struct kretprobe_reg probes[] = {
	{ &dev_ioctl_krp, VPNHIDE_HOOK_DEV_IOCTL, false },
	{ &sock_ioctl_krp, VPNHIDE_HOOK_SOCK_IOCTL, false },
	{ &rtnl_fill_krp, VPNHIDE_HOOK_RTNL_FILL_IFINFO, false },
	{ &inet6_fill_krp, VPNHIDE_HOOK_INET6_FILL_IFADDR, false },
	{ &inet_fill_krp, VPNHIDE_HOOK_INET_FILL_IFADDR, false },
	{ &fib_route_krp, VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW, false },
	{ &ipv6_route_krp, VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW, false },
	{ &fib_dump_krp, VPNHIDE_HOOK_FIB_DUMP_INFO, false },
	{ &rt6_fill_krp, VPNHIDE_HOOK_RT6_FILL_NODE, false },
	{ &fib_rule_fill_krp, VPNHIDE_HOOK_FIB_NL_FILL_RULE, false },
};

/* Bitset of logical hooks that fully registered — the `status` hooks mask. */
static u32 installed_hook_mask(void)
{
	u32 mask = 0;
	int i;

	for (i = 0; i < ARRAY_SIZE(probes); i++)
		if (probes[i].registered)
			mask |= vpnhide_hook_bit(probes[i].hook_id);
	if (socket_bind_hooks_registered)
		mask |= vpnhide_hook_bit(VPNHIDE_HOOK_SOCKET_BIND_INTERFACE);
	if (filesystem_hooks_registered)
		mask |= vpnhide_hook_bit(VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS);
	return mask;
}

/* Human-readable RFC2863 operstate for the diagnostic dump below. Never
 * returns NULL. */
static const char *netdev_operstate_str(unsigned char operstate)
{
	switch (operstate) {
	case IF_OPER_UNKNOWN:
		return "unknown";
	case IF_OPER_NOTPRESENT:
		return "notpresent";
	case IF_OPER_DOWN:
		return "down";
	case IF_OPER_LOWERLAYERDOWN:
		return "lowerlayerdown";
	case IF_OPER_TESTING:
		return "testing";
	case IF_OPER_DORMANT:
		return "dormant";
	case IF_OPER_UP:
		return "up";
	}
	return "?";
}

/* ================================================================== */
/*  /proc/vpnhide_diag — read-only field-debugging dump                */
/*                                                                    */
/*  NOT part of the frozen control/telemetry wire (docs/protocol.md): */
/*  a separate proc node, plain human-readable text, never written to */
/*  and never parsed by the activator or app (grep confirms only      */
/*  /proc/vpnhide_ctl is read by crates/activator and the lsposed     */
/*  app's ShellUtils/DashboardData). Exists to surface kernel-internal */
/*  state /proc/vpnhide_ctl's status/stats lines don't carry: per-     */
/*  probe registration + kretprobe/kprobe miss counters (is a Xiaomi  */
/*  HyperOS device exhausting VPNHIDE_KRETPROBE_MAXACTIVE?), and the  */
/*  LIVE is_vpn_ifname() verdict against every netdev in the reader's */
/*  netns — for root-causing reports of unexpected filtering behaviour */
/*  (e.g. a false-positive match hiding a non-VPN interface and         */
/*  breaking that device's networking). Deliberately reports the       */
/*  CURRENT verdict as-is, bugs included: "fixing" the match here      */
/*  would hide the very mismatch this node exists to surface.          */
/* ================================================================== */

static int vpnhide_diag_show(struct seq_file *m, void *v)
{
	struct nsproxy *nsproxy;
	struct net *net;
	struct net_device *dev;
	int i;

	seq_puts(m, "vpnhide diag\n");
	seq_printf(m, "active_hook_mask 0x%x installed_hook_mask 0x%x\n",
		   READ_ONCE(active_hook_mask), installed_hook_mask());

	for (i = 0; i < ARRAY_SIZE(probes); i++) {
		struct kretprobe *krp = probes[i].krp;

		seq_printf(
			m,
			"kretprobe %s registered=%d nmissed=%lu kp_nmissed=%lu\n",
			krp->kp.symbol_name, probes[i].registered,
			(unsigned long)krp->nmissed,
			(unsigned long)krp->kp.nmissed);
	}

	for (i = 0; i < ARRAY_SIZE(socket_bind_kprobe_hooks); i++) {
		struct socket_bind_kprobe_hook *hook =
			&socket_bind_kprobe_hooks[i];

		seq_printf(m, "kprobe %s registered=%d nmissed=%lu\n",
			   hook->name, hook->registered,
			   (unsigned long)hook->kp.nmissed);
	}

	for (i = 0; i < ARRAY_SIZE(filesystem_kprobe_hooks); i++) {
		struct filesystem_kprobe_hook *hook =
			&filesystem_kprobe_hooks[i];

		seq_printf(m, "kprobe %s registered=%d nmissed=%lu\n",
			   hook->name, hook->registered,
			   (unsigned long)hook->kp.nmissed);
	}

	/*
	 * Live is_vpn_ifname() verdict over every netdev in the reader's
	 * (i.e. the root shell's) network namespace. This handler runs in
	 * process context off seq_read(), not inside a probe handler, so
	 * rcu_read_lock() + for_each_netdev_rcu() is the ordinary safe way
	 * to walk the netdev list here — unlike the probe entry/return
	 * handlers above, which run with interrupts/preemption states that
	 * rule it out.
	 */
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

		if (!copy_dev_name(dev, name))
			continue;
		seq_printf(
			m,
			"iface %s ifindex=%d is_vpn=%d up=%d operstate=%s flags=0x%x\n",
			name, dev->ifindex, is_vpn_ifname(name) ? 1 : 0,
			!!(dev->flags & IFF_UP),
			netdev_operstate_str(dev->operstate), dev->flags);
	}
	rcu_read_unlock();

	return 0;
}

static int __init vpnhide_init(void)
{
	int i, ret, ok = 0;

	for (i = 0; i < ARRAY_SIZE(probes); i++) {
		ret = register_kretprobe(probes[i].krp);
		if (ret < 0) {
			pr_warn(MODNAME ": kretprobe(%s) failed: %d\n",
				probes[i].krp->kp.symbol_name, ret);
		} else {
			probes[i].registered = true;
			ok++;
			pr_info(MODNAME ": kretprobe(%s) registered\n",
				probes[i].krp->kp.symbol_name);
		}
	}

	if (ok == 0) {
		pr_err(MODNAME ": no kretprobes registered, aborting\n");
		return -ENOENT;
	}
	if (ok < ARRAY_SIZE(probes))
		pr_warn(MODNAME ": only %d/%zu kretprobes registered — "
				"some detection paths are not covered\n",
			ok, ARRAY_SIZE(probes));
	/* 0600: root-only read/write. The config snapshot is written here by
	 * service.sh and the VPN Hide app (both root). Apps must not see the
	 * control channel. (Renamed from vpnhide_targets for semantic accuracy
	 * — it is now control+stats, not just targets, §OPEN-4.) */
	if (!proc_create("vpnhide_ctl", 0600, NULL, &ctl_proc_ops)) {
		/* Without /proc/vpnhide_ctl userspace cannot configure the
		 * target list, so the module would silently filter nothing —
		 * fail loudly instead of pretending to work. */
		pr_err(MODNAME ": proc_create(vpnhide_ctl) failed; aborting\n");
		for (i = 0; i < ARRAY_SIZE(probes); i++)
			if (probes[i].registered)
				unregister_kretprobe(probes[i].krp);
		return -ENOMEM;
	}

	/*
	 * Diagnostic-only node — best effort. Its absence must not abort
	 * module init: unlike vpnhide_ctl it exposes no control surface, only
	 * human-readable internal state for field debugging (see the comment
	 * above vpnhide_diag_show). 0400: root-only read, matching
	 * vpnhide_ctl's root-only owner but read-only since there is nothing
	 * to write.
	 */
	if (!proc_create_single("vpnhide_diag", 0400, NULL, vpnhide_diag_show))
		pr_warn(MODNAME
			": proc_create_single(vpnhide_diag) failed — diagnostics unavailable\n");

	/* Resolve dev_get_by_index_rcu through a kprobe before the bind hooks go
	 * live: the symbol is missing from some OEMs' trimmed GKI KMI module table,
	 * so it must not be a hard link-time reference. If it stays NULL,
	 * classify_bind_ifindex fails closed rather than crashing. */
	if (!resolved_dev_get_by_index_rcu)
		resolved_dev_get_by_index_rcu =
			(dev_get_by_index_rcu_fn)resolve_kernel_symbol(
				"dev_get_by_index_rcu");

	/* Activate execution redirection last. No fallible initialization may run
	 * after module text becomes reachable outside the kprobe handler. The module
	 * has no exit function and remains resident until reboot. */
	register_socket_bind_hooks();
	if (READ_ONCE(filesystem_hiding))
		register_filesystem_hooks();

	pr_info(MODNAME
		": loaded — filesystem_hiding=%d; write a config snapshot to /proc/vpnhide_ctl\n",
		filesystem_hiding ? 1 : 0);
	return 0;
}

module_init(vpnhide_init);
/* Deliberately no module_exit(): vpnhide_kmod stays loaded until reboot. */

/* The source is MIT-licensed (see SPDX header), but MODULE_LICENSE("GPL")
 * is required to resolve EXPORT_SYMBOL_GPL symbols (kretprobes, etc.)
 * at module load time. */
MODULE_LICENSE("GPL");
MODULE_AUTHOR("okhsunrog");
MODULE_DESCRIPTION("Hide VPN interfaces from selected apps at kernel level");
