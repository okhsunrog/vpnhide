// SPDX-License-Identifier: MIT
/*
 * vpnhide (in-tree) — interface/address/route predicate hooks.
 *
 * One predicate covers every "is this netdev/name a hidden VPN interface for the
 * caller" site: the per-interface ioctls (dev_ifsioc / dev_ifname), the
 * SIOCGIFCONF enumeration (dev_ifconf), the RTM_* dump fills (rtnl_fill_ifinfo,
 * inet{,6}_fill_ifaddr), the /proc/net route show paths (fib_route_seq_show,
 * ipv6_route_seq_show), the RTM_GETROUTE fills (fib_dump_info, rt6_fill_node),
 * and the policy-rule fill (fib_nl_fill_rule). The per-version patch places
 * `if (vpnhide_should_hide_*(...)) return <skip>;` at the top of each, passing
 * the hook id so per-hook masks and stats are preserved.
 *
 * The .ko does the same filtering from kretprobe return handlers; here it is a
 * direct call, so there is no pt_regs unpacking and no kretprobe pool to exhaust.
 */

#include <linux/kernel.h>
#include <linux/string.h>
#include <linux/if.h>
#include <linux/netdevice.h>

#include <linux/vpnhide.h>

#include "vpnhide_internal.h"
#include "generated/iface_lists.h"
#include "shared/vpnhide_logic.h"

#define is_vpn_ifname(name) vpnhide_iface_is_vpn(name)

bool vpnhide_should_hide_ifname(const char *ifname, int hook_id)
{
	char name[IFNAMSIZ];

	if (!ifname)
		return false;
	if (!vpnhide_hook_active((enum vpnhide_hook_id)hook_id))
		return false;

	strscpy(name, ifname, sizeof(name));
	if (!is_vpn_ifname(name))
		return false;

	vpnhide_dbg("hide ifname=%s hook=%d\n", name, hook_id);
	vpnhide_record_hook_hit((enum vpnhide_hook_id)hook_id);
	return true;
}

bool vpnhide_should_hide_dev(const struct net_device *dev, int hook_id)
{
	if (!dev)
		return false;
	return vpnhide_should_hide_ifname(dev->name, hook_id);
}
