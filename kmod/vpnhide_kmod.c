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
 *   - dev_ioctl: filters SIOCGIFFLAGS / SIOCGIFNAME
 *   - dev_ifconf: filters SIOCGIFCONF interface enumeration
 *   - rtnl_fill_ifinfo: filters RTM_NEWLINK netlink dumps (getifaddrs)
 *   - inet6_fill_ifaddr: filters RTM_GETADDR IPv6 responses (getifaddrs)
 *   - inet_fill_ifaddr: filters RTM_GETADDR IPv4 responses (getifaddrs)
 *   - fib_route_seq_show: filters /proc/net/route entries
 *
 * Target UIDs are written to /proc/vpnhide_targets from userspace.
 */

#include <linux/module.h>
#include <linux/kernel.h>
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

#define MODNAME "vpnhide"
#define MAX_TARGET_UIDS 64

/* ------------------------------------------------------------------ */
/*  VPN interface name matching                                       */
/* ------------------------------------------------------------------ */

static const char *const vpn_prefixes[] = {
	"tun", "ppp", "tap", "wg", "ipsec", "xfrm", "utun", "l2tp", "gre",
};

static bool is_vpn_ifname(const char *name)
{
	int i;

	if (!name || !*name)
		return false;

	for (i = 0; i < ARRAY_SIZE(vpn_prefixes); i++) {
		if (strncmp(name, vpn_prefixes[i], strlen(vpn_prefixes[i])) ==
		    0)
			return true;
	}
	if (strstr(name, "vpn") || strstr(name, "VPN"))
		return true;

	return false;
}

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
};

static int dev_ioctl_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct dev_ioctl_data *data = (void *)ri->data;

	data->cmd = (unsigned int)regs->regs[1];
	data->kifr = (struct ifreq *)regs->regs[2];

	if (!is_target_uid())
		data->cmd = 0;

	return 0;
}

static int dev_ioctl_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct dev_ioctl_data *data = (void *)ri->data;
	char name[IFNAMSIZ];

	if (data->cmd == 0 || regs_return_value(regs) != 0)
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

	if (is_vpn_ifname(name))
		regs_set_return_value(regs, -ENODEV);

	return 0;
}

static struct kretprobe dev_ioctl_krp = {
	.handler = dev_ioctl_ret,
	.entry_handler = dev_ioctl_entry,
	.data_size = sizeof(struct dev_ioctl_data),
	.maxactive = 20,
	.kp.symbol_name = "dev_ioctl",
};

/* ================================================================== */
/*  Hook 2: dev_ifconf — SIOCGIFCONF interface enumeration            */
/*                                                                    */
/*  dev_ifconf(struct net *net, struct ifconf __user *uifc)           */
/*  arm64: x0=net, x1=uifc (__user pointer)                          */
/*                                                                    */
/*  After dev_ifconf returns, the userspace buffer contains ifreq     */
/*  entries. We compact out VPN entries and update ifc_len.           */
/* ================================================================== */

struct dev_ifconf_data {
	struct ifconf __user *uifc;
	bool target;
};

static int dev_ifconf_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct dev_ifconf_data *data = (void *)ri->data;

	data->uifc = (struct ifconf __user *)regs->regs[1];
	data->target = is_target_uid();
	return 0;
}

static int dev_ifconf_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct dev_ifconf_data *data = (void *)ri->data;
	struct ifconf ifc;
	struct ifreq __user *usr_ifr;
	struct ifreq tmp;
	int i, n, dst;

	if (!data->target || regs_return_value(regs) != 0 || !data->uifc)
		return 0;

	if (copy_from_user(&ifc, data->uifc, sizeof(ifc)))
		return 0;
	if (!ifc.ifc_req || ifc.ifc_len <= 0)
		return 0;

	n = ifc.ifc_len / (int)sizeof(struct ifreq);
	usr_ifr = ifc.ifc_req;
	dst = 0;

	for (i = 0; i < n; i++) {
		if (copy_from_user(&tmp, &usr_ifr[i], sizeof(tmp)))
			return 0; /* copy failed — leave buffer untouched */
		tmp.ifr_name[IFNAMSIZ - 1] = '\0';
		if (is_vpn_ifname(tmp.ifr_name))
			continue;
		if (dst != i) {
			if (copy_to_user(&usr_ifr[dst], &tmp, sizeof(tmp)))
				return 0; /* copy failed — stop compacting */
		}
		dst++;
	}

	if (dst < n) {
		ifc.ifc_len = dst * (int)sizeof(struct ifreq);
		/* dev_ifconf writes ifc_len via put_user, so we overwrite */
		if (put_user(ifc.ifc_len, &data->uifc->ifc_len))
			return 0;
	}

	return 0;
}

static struct kretprobe dev_ifconf_krp = {
	.handler = dev_ifconf_ret,
	.entry_handler = dev_ifconf_entry,
	.data_size = sizeof(struct dev_ifconf_data),
	.maxactive = 20,
	.kp.symbol_name = "dev_ifconf",
};

/* ================================================================== */
/*  Hook 3: rtnl_fill_ifinfo — netlink RTM_NEWLINK (getifaddrs path)  */
/*                                                                    */
/*  rtnl_fill_ifinfo fills one interface's data into a netlink skb    */
/*  during a RTM_GETLINK dump. If the device is a VPN and the caller  */
/*  is a target UID, we make it return -EMSGSIZE which tells the      */
/*  dump iterator to skip this entry (it thinks the skb is full for   */
/*  this entry and moves on, but the entry never gets added).         */
/* ================================================================== */

struct rtnl_fill_data {
	bool should_filter;
};

static int rtnl_fill_entry(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rtnl_fill_data *data = (void *)ri->data;
	struct net_device *dev;

	data->should_filter = false;

	if (!is_target_uid())
		return 0;

	/*
	 * rtnl_fill_ifinfo(struct sk_buff *skb, struct net_device *dev, ...)
	 * arm64: x0=skb, x1=dev
	 */
	dev = (struct net_device *)regs->regs[1];
	/* Callers hold RTNL which protects dev->name, but take RCU as
	 * belt-and-suspenders — same rationale as inet6_fill_entry. */
	rcu_read_lock();
	if (dev && is_vpn_ifname(dev->name))
		data->should_filter = true;
	rcu_read_unlock();

	return 0;
}

static int rtnl_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct rtnl_fill_data *data = (void *)ri->data;

	if (data->should_filter)
		regs_set_return_value(regs, -EMSGSIZE);

	return 0;
}

static struct kretprobe rtnl_fill_krp = {
	.handler = rtnl_fill_ret,
	.entry_handler = rtnl_fill_entry,
	.data_size = sizeof(struct rtnl_fill_data),
	.maxactive = 20,
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
	}
	rcu_read_unlock();

	return 0;
}

static int inet6_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct inet6_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	/* Undo whatever the fill function wrote to the skb */
	skb_trim(data->skb, data->saved_len);
	regs_set_return_value(regs, 0);
	return 0;
}

static struct kretprobe inet6_fill_krp = {
	.handler = inet6_fill_ret,
	.entry_handler = inet6_fill_entry,
	.data_size = sizeof(struct inet6_fill_data),
	.maxactive = 20,
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
	}
	rcu_read_unlock();

	return 0;
}

static int inet_fill_ret(struct kretprobe_instance *ri, struct pt_regs *regs)
{
	struct inet_fill_data *data = (void *)ri->data;

	if (!data->should_filter || !data->skb)
		return 0;

	skb_trim(data->skb, data->saved_len);
	regs_set_return_value(regs, 0);
	return 0;
}

static struct kretprobe inet_fill_krp = {
	.handler = inet_fill_ret,
	.entry_handler = inet_fill_entry,
	.data_size = sizeof(struct inet_fill_data),
	.maxactive = 20,
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

	if (data->target && data->seq)
		data->start_count = data->seq->count;
	else
		data->start_count = 0;

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
	.maxactive = 20,
	.kp.symbol_name = "fib_route_seq_show",
};

/* ================================================================== */
/*  Module init / exit                                                */
/* ================================================================== */

static struct proc_dir_entry *targets_entry;

struct kretprobe_reg {
	struct kretprobe *krp;
	const char *name;
	bool registered;
};

static struct kretprobe_reg probes[] = {
	{ &dev_ioctl_krp, "dev_ioctl", false },
	{ &dev_ifconf_krp, "dev_ifconf", false },
	{ &rtnl_fill_krp, "rtnl_fill_ifinfo", false },
	{ &inet6_fill_krp, "inet6_fill_ifaddr", false },
	{ &inet_fill_krp, "inet_fill_ifaddr", false },
	{ &fib_route_krp, "fib_route_seq_show", false },
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

	pr_info(MODNAME ": loaded — write UIDs to /proc/vpnhide_targets\n");
	return 0;
}

static void __exit vpnhide_exit(void)
{
	int i;

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
