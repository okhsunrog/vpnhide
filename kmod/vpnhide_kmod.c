// SPDX-License-Identifier: MIT
/*
 * vpnhide_kmod — kernel module that hides VPN network interfaces from
 * selected Android apps by filtering ioctl, netlink, and procfs
 * responses based on the calling process's UID.
 *
 * Uses kretprobes so no modification of the running kernel is needed;
 * works on stock Android GKI kernels with CONFIG_KPROBES=y.
 *
 * Hooks:
 *   - dev_ioctl: filters SIOCGIFFLAGS / SIOCGIFNAME / SIOCGIFMTU / etc.
 *   - sock_ioctl: filters SIOCGIFCONF interface enumeration
 *   - rtnl_fill_ifinfo: filters RTM_NEWLINK netlink dumps (getifaddrs)
 *   - inet6_fill_ifaddr: filters RTM_GETADDR IPv6 responses (getifaddrs)
 *   - inet_fill_ifaddr: filters RTM_GETADDR IPv4 responses (getifaddrs)
 *   - fib_route_seq_show: filters /proc/net/route entries
 *
 * Target UIDs are written to /proc/vpnhide_targets from userspace.
 *
 * Architecture: arm64 only. The handlers read syscall arguments via
 * `regs->regs[N]` (AAPCS64 calling convention). On other architectures
 * those slots have a different meaning, so the build is gated below.
 */

#include <linux/module.h>
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
#include <linux/netdevice.h>
#include <linux/rtnetlink.h>
#include <linux/skbuff.h>
#include <linux/inetdevice.h>
#include <net/if_inet6.h>
#include <net/ip_fib.h>
#include <net/ip6_fib.h>
#include <net/ip6_route.h>
#include <net/route.h>

#include "generated/iface_lists.h"

#ifndef CONFIG_ARM64
#error "vpnhide_kmod currently supports only arm64 (handlers read regs->regs[N] directly)"
#endif

#define MODNAME "vpnhide"
#define MAX_TARGET_UIDS 64

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
 * meaningful memory: 6 probes × 64 instances × ~80 B ≈ 30 KB total.
 */
#define VPNHIDE_KRETPROBE_MAXACTIVE 64

/* ------------------------------------------------------------------ */
/*  Debug logging — toggled via /proc/vpnhide_debug                   */
/* ------------------------------------------------------------------ */

static bool debug_enabled;

/*
 * `debug_enabled` is a single bool, written from /proc/vpnhide_debug
 * and read from every probe handler. Use READ_ONCE/WRITE_ONCE so the
 * compiler doesn't tear the access or hoist it across the probe-hot
 * path — kosher kernel style for unsynchronised flags.
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

struct vpnhide_targets {
	uid_t uids[MAX_TARGET_UIDS];
	int count;
	struct rcu_head rcu;
};

static struct vpnhide_targets __rcu *global_targets;
static DEFINE_SPINLOCK(targets_update_lock);

static struct vpnhide_targets __rcu *global_direct_targets;
static DEFINE_SPINLOCK(direct_targets_update_lock);

static bool is_target_uid(void)
{
	uid_t uid = from_kuid(&init_user_ns, current_uid());
	struct vpnhide_targets *t;
	bool found = false;
	int i;

	rcu_read_lock();
	t = rcu_dereference(global_targets);
	if (t) {
		for (i = 0; i < t->count; i++) {
			if (t->uids[i] == uid) {
				found = true;
				break;
			}
		}
	}
	rcu_read_unlock();
	return found;
}

static bool is_direct_target_uid(void)
{
	uid_t uid = from_kuid(&init_user_ns, current_uid());
	struct vpnhide_targets *t;
	bool found = false;
	int i;

	rcu_read_lock();
	t = rcu_dereference(global_direct_targets);
	if (t) {
		for (i = 0; i < t->count; i++) {
			if (t->uids[i] == uid) {
				found = true;
				break;
			}
		}
	}
	rcu_read_unlock();
	return found;
}

/* ------------------------------------------------------------------ */
/*  /proc/vpnhide_targets                                             */
/* ------------------------------------------------------------------ */

static void free_targets_rcu(struct rcu_head *rcu)
{
	struct vpnhide_targets *t = container_of(rcu, struct vpnhide_targets, rcu);
	kfree(t);
}

static ssize_t targets_write(struct file *file, const char __user *ubuf,
			     size_t count, loff_t *ppos)
{
	char *buf, *line, *next;
	struct vpnhide_targets *new_t, *old_t;
	int new_count = 0;

	if (count > PAGE_SIZE)
		return -EINVAL;

	buf = kmalloc(count + 1, GFP_KERNEL);
	if (!buf)
		return -ENOMEM;

	if (copy_from_user(buf, ubuf, count)) {
		kfree(buf);
		return -EFAULT;
	}
	buf[count] = '\0';

	new_t = kzalloc(sizeof(*new_t), GFP_KERNEL);
	if (!new_t) {
		kfree(buf);
		return -ENOMEM;
	}

	for (line = buf; line && *line && new_count < MAX_TARGET_UIDS;
	     line = next) {
		unsigned long uid;

		next = strchr(line, '\n');
		if (next)
			*next++ = '\0';

		while (*line == ' ' || *line == '\t')
			line++;
		if (!*line || *line == '#')
			continue;

		if (kstrtoul(line, 10, &uid) == 0)
			new_t->uids[new_count++] = (uid_t)uid;
	}
	new_t->count = new_count;

	spin_lock(&targets_update_lock);
	old_t = rcu_dereference_protected(global_targets,
					  lockdep_is_held(&targets_update_lock));
	rcu_assign_pointer(global_targets, new_t);
	spin_unlock(&targets_update_lock);

	if (old_t)
		call_rcu(&old_t->rcu, free_targets_rcu);

	kfree(buf);
	pr_info(MODNAME ": loaded %d target UIDs\n", new_count);
	return count;
}

static int targets_show(struct seq_file *m, void *v)
{
	struct vpnhide_targets *t;
	int i;

	rcu_read_lock();
	t = rcu_dereference(global_targets);
	if (t) {
		for (i = 0; i < t->count; i++)
			seq_printf(m, "%u\n", t->uids[i]);
	}
	rcu_read_unlock();
	return 0;
}

static int targets_open(struct inode *inode, struct file *file)
{
	return single_open(file, targets_show, NULL);
}

static const struct proc_ops targets_proc_ops = {
	.proc_open = targets_open,
	.proc_read = seq_read,
	.proc_write = targets_write,
	.proc_lseek = seq_lseek,
	.proc_release = single_release,
};

/* ------------------------------------------------------------------ */
/*  /proc/vpnhide_direct_targets                                      */
/* ------------------------------------------------------------------ */

static ssize_t direct_targets_write(struct file *file, const char __user *ubuf,
				    size_t count, loff_t *ppos)
{
	char *buf, *line, *next;
	struct vpnhide_targets *new_t, *old_t;
	int new_count = 0;

	if (count > PAGE_SIZE)
		return -EINVAL;

	buf = kmalloc(count + 1, GFP_KERNEL);
	if (!buf)
		return -ENOMEM;

	if (copy_from_user(buf, ubuf, count)) {
		kfree(buf);
		return -EFAULT;
	}
	buf[count] = '\0';

	new_t = kzalloc(sizeof(*new_t), GFP_KERNEL);
	if (!new_t) {
		kfree(buf);
		return -ENOMEM;
	}

	for (line = buf; line && *line && new_count < MAX_TARGET_UIDS;
	     line = next) {
		unsigned long uid;
		next = strchr(line, '\n');
		if (next) *next++ = '\0';
		while (*line == ' ' || *line == '\t') line++;
		if (!*line || *line == '#') continue;
		if (kstrtoul(line, 10, &uid) == 0)
			new_t->uids[new_count++] = (uid_t)uid;
	}
	new_t->count = new_count;

	spin_lock(&direct_targets_update_lock);
	old_t = rcu_dereference_protected(global_direct_targets,
					  lockdep_is_held(&direct_targets_update_lock));
	rcu_assign_pointer(global_direct_targets, new_t);
	spin_unlock(&direct_targets_update_lock);

	if (old_t)
		call_rcu(&old_t->rcu, free_targets_rcu);

	kfree(buf);
	pr_info(MODNAME ": loaded %d direct target UIDs\n", new_count);
	return count;
}

static int direct_targets_show(struct seq_file *m, void *v)
{
	struct vpnhide_targets *t;
	int i;

	rcu_read_lock();
	t = rcu_dereference(global_direct_targets);
	if (t) {
		for (i = 0; i < t->count; i++)
			seq_printf(m, "%u\n", t->uids[i]);
	}
	rcu_read_unlock();
	return 0;
}

static int direct_targets_open(struct inode *inode, struct file *file)
{
	return single_open(file, direct_targets_show, NULL);
}

static const struct proc_ops direct_targets_proc_ops = {
	.proc_open = direct_targets_open,
	.proc_read = seq_read,
	.proc_write = direct_targets_write,
	.proc_lseek = seq_lseek,
	.proc_release = single_release,
};

/* ------------------------------------------------------------------ */
/*  /proc/vpnhide_debug                                               */
/* ------------------------------------------------------------------ */

static ssize_t debug_write(struct file *file, const char __user *ubuf,
			   size_t count, loff_t *ppos)
{
	char c;

	if (count == 0)
		return 0;
	if (get_user(c, ubuf))
		return -EFAULT;

	WRITE_ONCE(debug_enabled, c == '1' || c == 'Y' || c == 'y');
	pr_info(MODNAME ": debug %s\n",
		READ_ONCE(debug_enabled) ? "enabled" : "disabled");
	return count;
}

static int debug_show(struct seq_file *m, void *v)
{
	seq_printf(m, "%d\n", READ_ONCE(debug_enabled) ? 1 : 0);
	return 0;
}

static int debug_open(struct inode *inode, struct file *file)
{
	return single_open(file, debug_show, NULL);
}

static const struct proc_ops debug_proc_ops = {
	.proc_open = debug_open,
	.proc_read = seq_read,
	.proc_write = debug_write,
	.proc_lseek = seq_lseek,
	.proc_release = single_release,
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
	data->active = is_target_uid();

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
/*  Hook 2: sock_ioctl — SIOCGIFCONF interface enumeration            */
/*                                                                    */
/*  Why sock_ioctl instead of dev_ifconf?                             */
/*                                                                    */
/*  On GKI 5.10 kernels built with Clang LTO (all stock Android       */
/*  devices), the linker inlines dev_ifconf() into sock_do_ioctl().   */
/*  The symbol "dev_ifconf" stays in kallsyms as a dead stub, so      */
/*  kretprobe registration succeeds but the probe never fires.        */
/*  Confirmed by disassembly on Xiaomi 13 Lite (5.10.136) and Lenovo  */
/*  Legion 2 Pro (5.10.101): no `bl dev_ifconf` in sock_do_ioctl.    */
/*                                                                    */
/*  On 6.1+, SIOCGIFCONF was moved out of sock_do_ioctl() into       */
/*  sock_ioctl() directly (handled in the switch statement), so       */
/*  hooking sock_do_ioctl would miss it on newer kernels.             */
/*                                                                    */
/*  sock_ioctl is the correct hook point because:                     */
/*  1. It is the file_operations->unlocked_ioctl callback for socket  */
/*     fds — used as a function pointer, so LTO cannot inline it.     */
/*  2. ALL socket ioctls, including SIOCGIFCONF, pass through it on   */
/*     every kernel version (5.10 through 6.12+).                     */
/*  3. After sock_ioctl returns, the ifconf data (ifreq array +       */
/*     ifc_len) is already in userspace — we filter it uniformly via  */
/*     copy_from_user/copy_to_user regardless of kernel version.      */
/*                                                                    */
/*  sock_ioctl(struct file *file, unsigned int cmd, unsigned long arg) */
/*  arm64: x0=file, x1=cmd, x2=arg (__user ptr)                      */
/*                                                                    */
/*  Performance: entry handler checks cmd == SIOCGIFCONF first (one   */
/*  compare), then is_target_uid(). For all other ioctls, overhead    */
/*  is a single branch. SIOCGIFCONF is rare (once per getifaddrs).    */
/* ================================================================== */

struct sock_ioctl_data {
	void __user *argp;
	bool target;
};

static int sock_ioctl_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct sock_ioctl_data *data = (void *)ri->data;
	unsigned int cmd = (unsigned int)regs->regs[1];

	data->target = false;

	if (cmd != SIOCGIFCONF)
		return 0;
	if (!is_target_uid())
		return 0;

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
 * Faults are handled cleanly: if the user buffer was unmapped or
 * raced, the copy fails with -EFAULT and we report COPY_FAULT to the
 * caller, who skips the ifc_len rewrite to avoid a half-filtered
 * array (`buffer compacted, length unchanged`) escaping to userspace.
 */
enum filter_ifconf_result {
	FILTER_IFCONF_NO_CHANGE,
	FILTER_IFCONF_CHANGED,
	FILTER_IFCONF_COPY_FAULT,
};

/* Compact VPN entries out of the userspace ifreq array. The caller is
 * responsible for updating `ifc_len` only on FILTER_IFCONF_CHANGED. */
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
	*out_len = dst * (int)sizeof(struct ifreq);
	return FILTER_IFCONF_CHANGED;
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
	if (!ifc.ifc_req || ifc.ifc_len <= 0)
		return 0;

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

	if (!is_target_uid()) {
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

	if (!is_target_uid())
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

	if (!is_target_uid())
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
	data->target = is_target_uid();

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
	char *buf, *src, *dst, *end;
	char ifname[IFNAMSIZ];
	int j;

	if (!data->target || !seq || !seq->buf)
		return 0;

	if (seq->count <= data->start_count)
		return 0;

	/*
	 * Scan the region [start_count, seq->count) for lines whose
	 * first tab-separated field is a VPN interface name. Compact
	 * out matching lines in place and adjust seq->count.
	 *
	 * Each route line looks like: "tun0\t08000000\t...\n"
	 */
	buf = seq->buf;
	src = buf + data->start_count;
	dst = src;
	end = buf + seq->count;

	while (src < end) {
		char *nl = memchr(src, '\n', end - src);
		char *line_end = nl ? nl + 1 : end;
		size_t line_len = line_end - src;

		/* Extract the interface name (first field, tab-delimited) */
		for (j = 0; j < IFNAMSIZ - 1 && j < (int)line_len &&
			    src[j] != '\t' && src[j] != '\n';
		     j++)
			ifname[j] = src[j];
		ifname[j] = '\0';

		if (is_vpn_ifname(ifname)) {
			vpnhide_dbg("fib_route_ret: hiding route for %s\n",
				    ifname);
			/* Skip this line */
			src = line_end;
			continue;
		}

		/* Keep this line — move it down if there's a gap */
		if (dst != src)
			memmove(dst, src, line_len);
		dst += line_len;
		src = line_end;
	}

	seq->count = dst - buf;
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
/*  Hook 7: fib_dump_info — IPv4 routes dump                          */
/*                                                                    */
/*  fib_dump_info(skb, portid, seq, event, fri, flags)                */
/*  arm64: x0=skb, x4=fri (struct fib_rt_info*)                       */
/* ================================================================== */

struct fib_dump_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static int fib_dump_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct fib_dump_data *data = (void *)ri->data;
	struct fib_info *fi = NULL;

	data->should_filter = false;

	if (!is_target_uid())
		return 0;

	/* x0=skb, x4=fi (or fri in 6.1+) */
	data->skb = (struct sk_buff *)regs->regs[0];

#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
	{
		struct fib_rt_info *fri = (struct fib_rt_info *)regs->regs[4];
		if (fri)
			fi = fri->fi;
	}
#else
	fi = (struct fib_info *)regs->regs[4];
#endif

	rcu_read_lock();
	if (fi && fi->fib_nhs > 0) {
		/* Access first nexthop interface */
		struct net_device *dev = fi->fib_nh[0].nh_common.nhc_dev;
		if (dev && is_vpn_ifname(dev->name)) {
			data->saved_len = data->skb ? data->skb->len : 0;
			data->should_filter = true;
			vpnhide_dbg("fib_dump_entry: hiding route via %s\n",
				    dev->name);
		}
	}
	rcu_read_unlock();

	return 0;
}

static int fib_dump_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct fib_dump_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	if (regs_return_value(regs) >= 0) {
		skb_trim(data->skb, data->saved_len);
		regs_set_return_value(regs, 0);
	}
	return 0;
}

static struct kretprobe fib_dump_krp = {
	.handler = fib_dump_ret,
	.entry_handler = fib_dump_entry,
	.data_size = sizeof(struct fib_dump_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "fib_dump_info",
};

/* ================================================================== */
/*  Hook 8: rt6_fill_node — IPv6 routes                                */
/*                                                                    */
/*  rt6_fill_node(net, skb, rt, dst, ...)                             */
/*  arm64: x1=skb, x3=dst (struct dst_entry*)                         */
/* ================================================================== */

struct rt6_fill_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static int rt6_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rt6_fill_data *data = (void *)ri->data;
	struct dst_entry *dst;

	data->should_filter = false;

	if (!is_target_uid())
		return 0;

	/* x1=skb, x3=dst */
	data->skb = (struct sk_buff *)regs->regs[1];
	dst = (struct dst_entry *)regs->regs[3];

	rcu_read_lock();
	if (dst && dst->dev && is_vpn_ifname(dst->dev->name)) {
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg("rt6_fill_entry: hiding IPv6 route via %s\n",
			    dst->dev->name);
	}
	rcu_read_unlock();

	return 0;
}

static int rt6_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rt6_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	if (regs_return_value(regs) >= 0) {
		skb_trim(data->skb, data->saved_len);
		regs_set_return_value(regs, 0);
	}
	return 0;
}

static struct kretprobe rt6_fill_krp = {
	.handler = rt6_fill_ret,
	.entry_handler = rt6_fill_entry,
	.data_size = sizeof(struct rt6_fill_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "rt6_fill_node",
};

/* ================================================================== */
/*  Hook 9: rt_fill_info — IPv4 single route lookup                   */
/*                                                                    */
/*  rt_fill_info(skb, dst, src, rt, ...)                              */
/*  arm64: x0=skb, x3=rt (struct rtable*)                             */
/* ================================================================== */

struct rt_fill_data {
	struct sk_buff *skb;
	unsigned int saved_len;
	bool should_filter;
};

static int rt_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rt_fill_data *data = (void *)ri->data;
	struct net_device *dev = NULL;

	data->should_filter = false;

	if (!is_target_uid())
		return 0;

#if LINUX_VERSION_CODE >= KERNEL_VERSION(6, 1, 0)
	/* GKI 6.1+ mapping: x0=skb, x3=fri */
	data->skb = (struct sk_buff *)regs->regs[0];
	{
		struct fib_rt_info *fri = (struct fib_rt_info *)regs->regs[3];
		if (fri && fri->fi && fri->fi->fib_nhs > 0)
			dev = fri->fi->fib_nh[0].nh_common.nhc_dev;
	}
#else
	/* GKI 5.10 / 5.15 mapping: x6=skb, x7=rt */
	data->skb = (struct sk_buff *)regs->regs[6];
	{
		struct rtable *rt = (struct rtable *)regs->regs[7];
		if (rt)
			dev = rt->dst.dev;
	}
#endif

	rcu_read_lock();
	if (dev && is_vpn_ifname(dev->name)) {
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg("rt_fill_entry: hiding route via %s\n", dev->name);
	}
	rcu_read_unlock();

	return 0;
}

static int rt_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rt_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	if (regs_return_value(regs) >= 0) {
		skb_trim(data->skb, data->saved_len);
		regs_set_return_value(regs, 0);
	}
	return 0;
}

static struct kretprobe rt_fill_krp = {
	.handler = rt_fill_ret,
	.entry_handler = rt_fill_entry,
	.data_size = sizeof(struct rt_fill_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "rt_fill_info",
};

/* ================================================================== */
/*  Optimized Routing State Cache                                     */
/* ================================================================== */

struct vpnhide_cache {
	bool vpn_active;
	int direct_idx;
	unsigned long last_update;
};

static struct vpnhide_cache routing_cache;
static DEFINE_SPINLOCK(cache_lock);

#define CACHE_TTL (HZ) /* 1 second */

static void update_routing_cache(struct net *net, struct vpnhide_cache *c)
{
	struct net_device *dev;
	int best_idx = 0;
	int current_prio = -1;
	bool vpn_found = false;

	rcu_read_lock();
	for_each_netdev_rcu(net, dev) {
		bool up = !!(dev->flags & IFF_UP);
		bool vpn = is_vpn_ifname(dev->name);

		if (up && vpn)
			vpn_found = true;

		if (up && !vpn && !(dev->flags & IFF_LOOPBACK) &&
		    !(dev->flags & IFF_POINTOPOINT)) {
			int prio = 0;
			if (vpnhide_iface_starts_with_ci(dev->name, "wlan"))
				prio = 10;
			else if (vpnhide_iface_starts_with_ci(dev->name, "rmnet") ||
				 vpnhide_iface_starts_with_ci(dev->name, "ccmni") ||
				 vpnhide_iface_starts_with_ci(dev->name, "pdp"))
				prio = 5;

			if (prio > current_prio) {
				best_idx = dev->ifindex;
				current_prio = prio;
			}
		}
	}
	rcu_read_unlock();

	c->vpn_active = vpn_found;
	c->direct_idx = best_idx;
	c->last_update = jiffies;
}

static void get_routing_info(struct net *net, bool *vpn_active, int *direct_idx)
{
	unsigned long now = jiffies;

	/* We use a simple spinlock for the cache update. Since it only happens
	 * once per second, there is no meaningful contention. */
	if (time_after(now, READ_ONCE(routing_cache.last_update) + CACHE_TTL)) {
		if (spin_trylock(&cache_lock)) {
			update_routing_cache(net, &routing_cache);
			spin_unlock(&cache_lock);
		}
	}

	*vpn_active = READ_ONCE(routing_cache.vpn_active);
	*direct_idx = READ_ONCE(routing_cache.direct_idx);
}

/* ================================================================== */
/*  Hook 10: ip_route_output_flow — IPv4 routing bypass               */
/*                                                                    */
/*  ip_route_output_flow(net, flp4, sk)                               */
/*  arm64: x1=flp4 (struct flowi4*)                                  */
/*                                                                    */
/*  For target UIDs, we set the 'protectedFromVpn' bit in the mark.   */
/*  On Android, this bit (typically 1 << 17) tells the routing        */
/*  rules to skip VPN tables and use physical ones.                   */
/* ================================================================== */

#define ANDROID_PROTECTED_FROM_VPN_BIT (1 << 17)
#define ANDROID_EXPLICITLY_SELECTED_BIT (1 << 16)

static int ip_route_output_flow_entry(struct kretprobe_instance *ri,
				      struct pt_regs *regs)
{
	struct net *net = (struct net *)regs->regs[0];
	struct flowi4 *flp4 = (struct flowi4 *)regs->regs[1];

	if (flp4 && is_direct_target_uid()) {
		bool vpn_active;
		int phys_idx;

		get_routing_info(net, &vpn_active, &phys_idx);
		if (!vpn_active)
			return 0;

		/* Force marks to bypass VPN. */
		flp4->flowi4_mark &= ~0xFFFF;
		flp4->flowi4_mark |= (ANDROID_PROTECTED_FROM_VPN_BIT |
				      ANDROID_EXPLICITLY_SELECTED_BIT);
		flp4->saddr = 0;
		if (phys_idx > 0)
			flp4->flowi4_oif = phys_idx;

		vpnhide_dbg("ip_route_output_flow: forced bypass (oif=%d) for uid=%u\n",
			    flp4->flowi4_oif, from_kuid(&init_user_ns, current_uid()));
	}
	return 0;
}

static struct kretprobe ip_route_output_flow_krp = {
	.entry_handler = ip_route_output_flow_entry,
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "ip_route_output_flow",
};

/* __ip_route_output_key is the inner routing function. */
static int ip_route_output_key_entry(struct kretprobe_instance *ri,
				     struct pt_regs *regs)
{
	struct net *net = (struct net *)regs->regs[0];
	struct flowi4 *flp4 = (struct flowi4 *)regs->regs[1];

	if (flp4 && is_direct_target_uid()) {
		bool vpn_active;
		int phys_idx;

		get_routing_info(net, &vpn_active, &phys_idx);
		if (!vpn_active)
			return 0;

		flp4->flowi4_mark &= ~0xFFFF;
		flp4->flowi4_mark |= (ANDROID_PROTECTED_FROM_VPN_BIT |
				      ANDROID_EXPLICITLY_SELECTED_BIT);
		flp4->saddr = 0;
		if (phys_idx > 0)
			flp4->flowi4_oif = phys_idx;
		
		vpnhide_dbg("__ip_route_output_key: forced bypass (oif=%d) for uid=%u\n",
			    flp4->flowi4_oif, from_kuid(&init_user_ns, current_uid()));
	}
	return 0;
}

static struct kretprobe ip_route_output_key_krp = {
	.entry_handler = ip_route_output_key_entry,
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "__ip_route_output_key",
};

/* ================================================================== */
/*  Hook 11: ip6_route_output — IPv6 routing bypass                   */
/*                                                                    */
/*  ip6_route_output(net, sk, fl6)                                    */
/*  arm64: x2=fl6 (struct flowi6*)                                   */
/* ================================================================== */

static int ip6_route_output_entry(struct kretprobe_instance *ri,
				  struct pt_regs *regs)
{
	struct net *net = (struct net *)regs->regs[0];
	struct flowi6 *fl6 = (struct flowi6 *)regs->regs[2];

	if (fl6 && is_direct_target_uid()) {
		bool vpn_active;
		int phys_idx;

		get_routing_info(net, &vpn_active, &phys_idx);
		if (!vpn_active)
			return 0;

		fl6->flowi6_mark &= ~0xFFFF;
		fl6->flowi6_mark |= (ANDROID_PROTECTED_FROM_VPN_BIT |
				     ANDROID_EXPLICITLY_SELECTED_BIT);
		fl6->saddr = in6addr_any;
		if (phys_idx > 0)
			fl6->flowi6_oif = phys_idx;
		
		vpnhide_dbg("ip6_route_output: forced bypass (oif=%d) for uid=%u\n",
			    fl6->flowi6_oif, from_kuid(&init_user_ns, current_uid()));
	}
	return 0;
}

static struct kretprobe ip6_route_output_krp = {
	.entry_handler = ip6_route_output_entry,
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "ip6_route_output",
};

/* ================================================================== */
/*  Module init / exit                                                */
/* ================================================================== */

static struct proc_dir_entry *targets_entry;
static struct proc_dir_entry *direct_targets_entry;
static struct proc_dir_entry *debug_entry;

struct kretprobe_reg {
	struct kretprobe *krp;
	const char *name;
	bool registered;
};

static struct kretprobe_reg probes[] = {
	{ &dev_ioctl_krp, "dev_ioctl", false },
	{ &sock_ioctl_krp, "sock_ioctl", false },
	{ &rtnl_fill_krp, "rtnl_fill_ifinfo", false },
	{ &inet6_fill_krp, "inet6_fill_ifaddr", false },
	{ &inet_fill_krp, "inet_fill_ifaddr", false },
	{ &fib_route_krp, "fib_route_seq_show", false },
	{ &fib_dump_krp, "fib_dump_info", false },
	{ &rt6_fill_krp, "rt6_fill_node", false },
	{ &rt_fill_krp, "rt_fill_info", false },
	{ &ip_route_output_flow_krp, "ip_route_output_flow", false },
	{ &ip_route_output_key_krp, "__ip_route_output_key", false },
	{ &ip6_route_output_krp, "ip6_route_output", false },
};

static int __init vpnhide_init(void)
{
	int i, ret, ok = 0;

	/* Initialize RCU targets pointers */
	rcu_assign_pointer(global_targets, NULL);
	rcu_assign_pointer(global_direct_targets, NULL);

	for (i = 0; i < ARRAY_SIZE(probes); i++) {
		ret = register_kretprobe(probes[i].krp);
		if (ret < 0) {
			pr_warn(MODNAME ": kretprobe(%s) failed: %d\n",
				probes[i].name, ret);
		} else {
			probes[i].registered = true;
			ok++;
			pr_info(MODNAME ": kretprobe(%s) registered\n",
				probes[i].name);
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

	/* 0600: root-only read/write. UIDs are written here by service.sh
	 * and the VPN Hide app (both root). Apps must not see the target list. */
	targets_entry =
		proc_create("vpnhide_targets", 0600, NULL, &targets_proc_ops);
	if (!targets_entry) {
		pr_err(MODNAME
		       ": proc_create(vpnhide_targets) failed; aborting\n");
		for (i = 0; i < ARRAY_SIZE(probes); i++)
			if (probes[i].registered)
				unregister_kretprobe(probes[i].krp);
		return -ENOMEM;
	}

	direct_targets_entry =
		proc_create("vpnhide_direct_targets", 0600, NULL, &direct_targets_proc_ops);
	if (!direct_targets_entry)
		pr_warn(MODNAME
			": proc_create(vpnhide_direct_targets) failed; direct routing configuration unavailable\n");

	debug_entry = proc_create("vpnhide_debug", 0600, NULL, &debug_proc_ops);
	if (!debug_entry)
		pr_warn(MODNAME
			": proc_create(vpnhide_debug) failed; debug toggle unavailable\n");

	pr_info(MODNAME ": loaded — write UIDs to /proc/vpnhide_targets and /proc/vpnhide_direct_targets\n");
	return 0;
}

static void __exit vpnhide_exit(void)
{
	struct vpnhide_targets *t;
	int i;

	if (debug_entry)
		proc_remove(debug_entry);
	if (targets_entry)
		proc_remove(targets_entry);
	if (direct_targets_entry)
		proc_remove(direct_targets_entry);

	for (i = 0; i < ARRAY_SIZE(probes); i++) {
		if (probes[i].registered) {
			unregister_kretprobe(probes[i].krp);
			pr_info(MODNAME ": kretprobe(%s) unregistered "
					"(missed %d)\n",
				probes[i].name, probes[i].krp->nmissed);
		}
	}

	/* Cleanup RCU targets */
	spin_lock(&targets_update_lock);
	t = rcu_dereference_protected(global_targets,
				      lockdep_is_held(&targets_update_lock));
	rcu_assign_pointer(global_targets, NULL);
	spin_unlock(&targets_update_lock);

	if (t) {
		synchronize_rcu();
		kfree(t);
	}

	/* Cleanup RCU direct targets */
	spin_lock(&direct_targets_update_lock);
	t = rcu_dereference_protected(global_direct_targets,
				      lockdep_is_held(&direct_targets_update_lock));
	rcu_assign_pointer(global_direct_targets, NULL);
	spin_unlock(&direct_targets_update_lock);

	if (t) {
		synchronize_rcu();
		kfree(t);
	}

	pr_info(MODNAME ": unloaded\n");
}

module_init(vpnhide_init);
module_exit(vpnhide_exit);

/* The source is MIT-licensed (see SPDX header), but MODULE_LICENSE("GPL")
 * is required to resolve EXPORT_SYMBOL_GPL symbols (kretprobes, etc.)
 * at module load time. */
MODULE_LICENSE("GPL");
MODULE_AUTHOR("okhsunrog");
MODULE_DESCRIPTION("Hide VPN interfaces from selected apps at kernel level");
