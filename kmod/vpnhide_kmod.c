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
 *   - ipv6_route_seq_show: filters /proc/net/ipv6_route entries
 *   - fib_dump_info / rt_fill_info: filter IPv4 RTM_GETROUTE replies
 *   - rt6_fill_node: filters IPv6 RTM_GETROUTE replies
 *   - fib_nl_fill_rule: filters policy routing rules for target UIDs
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
#include <net/nexthop.h>
#include <net/ip6_fib.h>
#include <net/ip6_route.h>
#include <net/route.h>
#include <net/fib_rules.h>

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

/* ------------------------------------------------------------------ */
/*  Target UID list                                                   */
/* ------------------------------------------------------------------ */

static uid_t target_uids[MAX_TARGET_UIDS];
static int nr_target_uids;
static DEFINE_SPINLOCK(uids_lock);

static bool is_target_uid(void)
{
	uid_t uid = from_kuid(&init_user_ns, current_uid());
	bool found = false;
	int i;

	spin_lock(&uids_lock);
	for (i = 0; i < nr_target_uids; i++) {
		if (target_uids[i] == uid) {
			found = true;
			break;
		}
	}
	spin_unlock(&uids_lock);
	return found;
}

/* ------------------------------------------------------------------ */
/*  /proc/vpnhide_targets                                             */
/* ------------------------------------------------------------------ */

static ssize_t targets_write(struct file *file, const char __user *ubuf,
			     size_t count, loff_t *ppos)
{
	char *buf, *line, *next;
	int new_count = 0;
	uid_t new_uids[MAX_TARGET_UIDS];

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
			new_uids[new_count++] = (uid_t)uid;
	}

	spin_lock(&uids_lock);
	memcpy(target_uids, new_uids, new_count * sizeof(uid_t));
	nr_target_uids = new_count;
	spin_unlock(&uids_lock);

	kfree(buf);
	pr_info(MODNAME ": loaded %d target UIDs\n", new_count);
	return count;
}

static int targets_show(struct seq_file *m, void *v)
{
	int i;

	spin_lock(&uids_lock);
	for (i = 0; i < nr_target_uids; i++)
		seq_printf(m, "%u\n", target_uids[i]);
	spin_unlock(&uids_lock);
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
	data->target = is_target_uid();

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
	char *buf, *src, *dst, *end;
	char ifname[IFNAMSIZ];
	int j;

	if (!data->target || !seq || !seq->buf)
		return 0;
	if (seq->count <= data->start_count)
		return 0;

	buf = seq->buf;
	src = buf + data->start_count;
	dst = src;
	end = buf + seq->count;

	while (src < end) {
		char *nl = memchr(src, '\n', end - src);
		char *line_end = nl ? nl + 1 : end;
		size_t line_len = line_end - src;
		char *field_start;
		char *field_end = line_end;

		while (field_end > src &&
		       (field_end[-1] == '\n' || field_end[-1] == '\r' ||
			field_end[-1] == ' ' || field_end[-1] == '\t'))
			field_end--;
		field_start = field_end;
		while (field_start > src && field_start[-1] != ' ' &&
		       field_start[-1] != '\t')
			field_start--;

		for (j = 0; j < IFNAMSIZ - 1 && (field_start + j) < field_end;
		     j++)
			ifname[j] = field_start[j];
		ifname[j] = '\0';

		if (is_vpn_ifname(ifname)) {
			vpnhide_dbg("ipv6_route_ret: hiding route for %s\n",
				    ifname);
			src = line_end;
			continue;
		}

		if (dst != src)
			memmove(dst, src, line_len);
		dst += line_len;
		src = line_end;
	}

	seq->count = dst - buf;
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

static bool is_physical_ifname(const char *name)
{
	return vpnhide_iface_starts_with_ci(name, "rmnet") ||
	       vpnhide_iface_starts_with_ci(name, "wlan") ||
	       vpnhide_iface_starts_with_ci(name, "eth") ||
	       vpnhide_iface_starts_with_ci(name, "ccmni") ||
	       vpnhide_iface_starts_with_ci(name, "ccemni") ||
	       vpnhide_iface_starts_with_ci(name, "seth");
}

static bool is_public_ipv4(__be32 addr)
{
	u32 host = be32_to_cpu(addr);
	u8 a = (host >> 24) & 0xff;
	u8 b = (host >> 16) & 0xff;
	u8 c = (host >> 8) & 0xff;

	if (a == 0 || a == 10 || a == 127 || a >= 224)
		return false;
	if (a == 100 && b >= 64 && b <= 127)
		return false;
	if (a == 169 && b == 254)
		return false;
	if (a == 172 && b >= 16 && b <= 31)
		return false;
	if (a == 192 && b == 168)
		return false;
	if (a == 192 && b == 0 && c == 0)
		return false;
	if (a == 192 && b == 0 && c == 2)
		return false;
	if (a == 198 && (b == 18 || b == 19))
		return false;
	if (a == 198 && b == 51 && c == 100)
		return false;
	if (a == 203 && b == 0 && c == 113)
		return false;
	return true;
}

static bool is_public_host_route_via_physical(const struct fib_rt_info *fri,
					      struct net_device *dev)
{
	char name[IFNAMSIZ];

	if (!fri || !dev || fri->dst_len != 32 || !is_public_ipv4(fri->dst))
		return false;
	if (!copy_dev_name(dev, name))
		return false;
	return is_physical_ifname(name);
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
			 const char *hook_name)
{
	if (!data->should_filter || !data->skb)
		return 0;

	if (regs_return_value(regs) >= 0) {
		vpnhide_dbg("%s: trimming skb %u -> %u\n", hook_name,
			    data->skb->len, data->saved_len);
		skb_trim(data->skb, data->saved_len);
		regs_set_return_value(regs, 0);
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

	if (!is_target_uid() || !fri)
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
	return route_skb_ret((void *)ri->data, regs, "fib_dump_ret");
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

	if (!is_target_uid())
		return 0;

	rcu_read_lock();
	dev = dev_from_fib6_info(rt);
	if (!dev && dst)
		copy_from_kernel_nofault(&dev, &dst->dev, sizeof(dev));
	if (copy_dev_name(dev, dev_name) && is_vpn_ifname(dev_name)) {
		data->skb = (struct sk_buff *)regs->regs[1];
		data->saved_len = data->skb ? data->skb->len : 0;
		data->should_filter = true;
		vpnhide_dbg("rt6_fill_entry: hiding route via %s\n",
			    dev_name);
	}
	rcu_read_unlock();

	return 0;
}

static int rt6_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	return route_skb_ret((void *)ri->data, regs, "rt6_fill_ret");
}

static struct kretprobe rt6_fill_krp = {
	.handler = rt6_fill_ret,
	.entry_handler = rt6_fill_entry,
	.data_size = sizeof(struct route_skb_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "rt6_fill_node",
};

/* ================================================================== */
/*  Hook 10: rt_fill_info — IPv4 single RTM_GETROUTE lookup           */
/*                                                                    */
/*  arm64 on current GKI: x3=rt (struct rtable*), x7=skb             */
/* ================================================================== */

static int rt_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct route_skb_data *data = (void *)ri->data;
	struct rtable *rt = (struct rtable *)regs->regs[3];
	struct net_device *dev = NULL;
	char dev_name[IFNAMSIZ];

	init_route_skb_data(data);

	if (!is_target_uid() || !rt)
		return 0;

	if (copy_from_kernel_nofault(&dev, &rt->dst.dev, sizeof(dev)) != 0 ||
	    !copy_dev_name(dev, dev_name) || !is_vpn_ifname(dev_name))
		return 0;

	data->skb = (struct sk_buff *)regs->regs[7];
	data->saved_len = data->skb ? data->skb->len : 0;
	data->should_filter = true;
	vpnhide_dbg("rt_fill_entry: hiding route via %s\n", dev_name);
	return 0;
}

static int rt_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	return route_skb_ret((void *)ri->data, regs, "rt_fill_ret");
}

static struct kretprobe rt_fill_krp = {
	.handler = rt_fill_ret,
	.entry_handler = rt_fill_entry,
	.data_size = sizeof(struct route_skb_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "rt_fill_info",
};

/* ================================================================== */
/*  Hook 11: fib_nl_fill_rule — RTM_GETRULE policy rules              */
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

	if (!is_target_uid() || !rule)
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
		uid_t end =
			from_kuid(&init_user_ns, rule_copy.uid_range.end);

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
	return route_skb_ret((void *)ri->data, regs, "fib_rule_fill_ret");
}

static struct kretprobe fib_rule_fill_krp = {
	.handler = fib_rule_fill_ret,
	.entry_handler = fib_rule_fill_entry,
	.data_size = sizeof(struct route_skb_data),
	.maxactive = VPNHIDE_KRETPROBE_MAXACTIVE,
	.kp.symbol_name = "fib_nl_fill_rule",
};

/* ================================================================== */
/*  Module init / exit                                                */
/* ================================================================== */

static struct proc_dir_entry *targets_entry;
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
	{ &ipv6_route_krp, "ipv6_route_seq_show", false },
	{ &fib_dump_krp, "fib_dump_info", false },
	{ &rt6_fill_krp, "rt6_fill_node", false },
	{ &rt_fill_krp, "rt_fill_info", false },
	{ &fib_rule_fill_krp, "fib_nl_fill_rule", false },
};

static int __init vpnhide_init(void)
{
	int i, ret, ok = 0;

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
		/* Without /proc/vpnhide_targets userspace cannot configure
		 * the target UID list, so the module would silently filter
		 * nothing — fail loudly instead of pretending to work. */
		pr_err(MODNAME
		       ": proc_create(vpnhide_targets) failed; aborting\n");
		for (i = 0; i < ARRAY_SIZE(probes); i++)
			if (probes[i].registered)
				unregister_kretprobe(probes[i].krp);
		return -ENOMEM;
	}
	debug_entry = proc_create("vpnhide_debug", 0600, NULL, &debug_proc_ops);
	if (!debug_entry)
		pr_warn(MODNAME
			": proc_create(vpnhide_debug) failed; debug toggle unavailable\n");

	pr_info(MODNAME ": loaded — write UIDs to /proc/vpnhide_targets\n");
	return 0;
}

static void __exit vpnhide_exit(void)
{
	int i;

	if (debug_entry)
		proc_remove(debug_entry);
	if (targets_entry)
		proc_remove(targets_entry);

	for (i = 0; i < ARRAY_SIZE(probes); i++) {
		if (probes[i].registered) {
			unregister_kretprobe(probes[i].krp);
			pr_info(MODNAME ": kretprobe(%s) unregistered "
					"(missed %d)\n",
				probes[i].name, probes[i].krp->nmissed);
		}
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
