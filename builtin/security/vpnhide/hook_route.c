// SPDX-License-Identifier: MIT
/*
 * vpnhide (in-tree) — route / policy-rule concealment.
 *
 * RTM_GETROUTE dumps (fib_dump_info, rt6_fill_node), /proc/net/{route,ipv6_route}
 * (fib_route_seq_show, ipv6_route_native_seq_show), and RTM_GETRULE
 * (fib_nl_fill_rule). Beyond hiding a VPN interface's own routes, the dump
 * predicates also drop a public host-route pinned to a physical uplink — the /32
 * or /128 a VPN client installs so tunnel packets reach the server, which leaks
 * the server's public IP even when the tun interface is hidden. The rule
 * predicate hides the target UID's policy routing rule.
 *
 * The .ko does all this from kretprobe return handlers reading raw pt_regs, so it
 * needs copy_from_kernel_nofault and hand-rolled nexthop walking. In-tree the
 * pointers are real and typed, so this uses the kernel's own fib_info_nhc() /
 * nexthop_fib6_nh() accessors directly. The address/iface predicates
 * (vpnhide_is_public_ipv{4,6}, vpnhide_iface_is_physical) are shared with the
 * .ko and KPM via shared/vpnhide_logic.h.
 */

#include <linux/kernel.h>
#include <linux/string.h>
#include <linux/if.h>
#include <linux/in6.h>
#include <linux/cred.h>
#include <linux/uidgid.h>
#include <linux/rcupdate.h>
#include <linux/netdevice.h>
#include <net/ip_fib.h>
#include <net/ip6_fib.h>
#include <net/nexthop.h>
#include <net/dst.h>
#include <net/fib_rules.h>
#include <uapi/linux/rtnetlink.h>

#include <linux/vpnhide.h>

#include "vpnhide_internal.h"
#include "generated/iface_lists.h"
#include "shared/vpnhide_logic.h"

#define is_vpn_ifname(name) vpnhide_iface_is_vpn(name)

/* ------------------------------------------------------------------ */
/*  Route dev extraction (kernel accessors, no fault-safe reads)      */
/* ------------------------------------------------------------------ */

static struct net_device *fib4_route_dev(const struct fib_info *fi)
{
	struct fib_nh_common *nhc;

	if (!fi)
		return NULL;
	nhc = fib_info_nhc((struct fib_info *)fi, 0);
	return nhc ? nhc->nhc_dev : NULL;
}

static struct net_device *fib6_route_dev(struct fib6_info *rt)
{
	struct fib6_nh *nh;

	if (!rt)
		return NULL;
	nh = rt->nh ? nexthop_fib6_nh(rt->nh) : rt->fib6_nh;
	return nh ? nh->fib_nh_dev : NULL;
}

/* A public /32 host-route pinned to a physical uplink leaks the VPN server's
 * IPv4 even when the tun is hidden. &fri->dst is the __be32's 4 network bytes. */
static bool is_public_host4_via_physical(const struct fib_rt_info *fri,
					 const struct net_device *dev)
{
	if (!fri || !dev || fri->dst_len != 32 ||
	    !vpnhide_is_public_ipv4((const unsigned char *)&fri->dst))
		return false;
	return vpnhide_iface_is_physical(dev->name);
}

/* IPv6 analogue: a public /128 host-route via a physical interface. */
static bool is_public_host6_via_physical(struct fib6_info *rt,
					 const struct net_device *dev)
{
	if (!rt || !dev || rt->fib6_dst.plen != 128 ||
	    !vpnhide_is_public_ipv6(rt->fib6_dst.addr.s6_addr))
		return false;
	return vpnhide_iface_is_physical(dev->name);
}

/* ------------------------------------------------------------------ */
/*  Predicates called from the patched emit functions                 */
/* ------------------------------------------------------------------ */

/* /proc/net/route line for a VPN interface. */
bool vpnhide_hide_fib_route(const struct fib_info *fi)
{
	struct net_device *dev;
	bool hide = false;

	if (!vpnhide_hook_active(VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW) || !fi)
		return false;
	rcu_read_lock();
	dev = fib4_route_dev(fi);
	if (dev && is_vpn_ifname(dev->name))
		hide = true;
	rcu_read_unlock();
	if (hide)
		vpnhide_record_hook_hit(VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW);
	return hide;
}

/* /proc/net/ipv6_route line for a VPN interface. */
bool vpnhide_hide_fib6_route(struct fib6_info *rt)
{
	struct net_device *dev;
	bool hide = false;

	if (!vpnhide_hook_active(VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW) || !rt)
		return false;
	rcu_read_lock();
	dev = fib6_route_dev(rt);
	if (dev && is_vpn_ifname(dev->name))
		hide = true;
	rcu_read_unlock();
	if (hide)
		vpnhide_record_hook_hit(VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW);
	return hide;
}

/* IPv4 RTM_GETROUTE reply: hide a VPN-iface route or a public host-route. */
bool vpnhide_hide_fib_dump(const struct fib_rt_info *fri)
{
	struct net_device *dev;
	bool hide = false;

	if (!vpnhide_hook_active(VPNHIDE_HOOK_FIB_DUMP_INFO) || !fri || !fri->fi)
		return false;
	rcu_read_lock();
	dev = fib4_route_dev(fri->fi);
	if (dev && (is_vpn_ifname(dev->name) ||
		    is_public_host4_via_physical(fri, dev)))
		hide = true;
	rcu_read_unlock();
	if (hide)
		vpnhide_record_hook_hit(VPNHIDE_HOOK_FIB_DUMP_INFO);
	return hide;
}

/* IPv6 RTM_GETROUTE reply: hide a VPN-iface route or a public host-route. */
bool vpnhide_hide_rt6(struct fib6_info *rt, struct dst_entry *dst)
{
	struct net_device *dev;
	bool hide = false;

	if (!vpnhide_hook_active(VPNHIDE_HOOK_RT6_FILL_NODE))
		return false;
	rcu_read_lock();
	dev = fib6_route_dev(rt);
	if (!dev && dst)
		dev = dst->dev;
	if (dev && (is_vpn_ifname(dev->name) ||
		    is_public_host6_via_physical(rt, dev)))
		hide = true;
	rcu_read_unlock();
	if (hide)
		vpnhide_record_hook_hit(VPNHIDE_HOOK_RT6_FILL_NODE);
	return hide;
}

/* RTM_GETRULE: hide a rule naming a VPN interface, or the target UID's policy
 * rule (a per-UID rule into a non-default table — how split routing exposes that
 * an app is being steered). Mirrors the .ko's fib_rule filter. */
bool vpnhide_hide_fib_rule(const struct fib_rule *rule)
{
	uid_t uid, start, end;

	if (!vpnhide_hook_active(VPNHIDE_HOOK_FIB_NL_FILL_RULE) || !rule)
		return false;

	if ((rule->iifname[0] && is_vpn_ifname(rule->iifname)) ||
	    (rule->oifname[0] && is_vpn_ifname(rule->oifname))) {
		vpnhide_record_hook_hit(VPNHIDE_HOOK_FIB_NL_FILL_RULE);
		return true;
	}

	uid = from_kuid(&init_user_ns, current_uid());
	start = from_kuid(&init_user_ns, rule->uid_range.start);
	end = from_kuid(&init_user_ns, rule->uid_range.end);
	if (uid >= start && uid <= end &&
	    (start != 0 || end != (uid_t)~0) &&
	    rule->table != RT_TABLE_MAIN &&
	    rule->table != RT_TABLE_LOCAL &&
	    rule->table != RT_TABLE_DEFAULT &&
	    rule->table > 100) {
		vpnhide_record_hook_hit(VPNHIDE_HOOK_FIB_NL_FILL_RULE);
		return true;
	}
	return false;
}
