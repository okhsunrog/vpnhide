/* SPDX-License-Identifier: MIT */
/* Offline count-walk ABI probe, never executed or loaded. Compile against the
 * actual prepared ARM64 kernel headers/configuration, then inspect the constant
 * .vpnhide_offsets section (nine little-endian u64s). This independently derives
 * the layout; it does not parse or include the KPM consumer table.
 * Field order matches the dated NULL-ifconf qualification record.
 */
#include <linux/kconfig.h>
#include <linux/netdevice.h>
#include <linux/inetdevice.h>
#include <linux/net.h>
#include <net/net_namespace.h>
#include <net/sock.h>

#if defined(CONFIG_GCC_PLUGIN_RANDSTRUCT) || defined(CONFIG_RANDSTRUCT)
#error "Randomized layouts cannot qualify a reference KPM table"
#endif
#ifndef CONFIG_NET_NS
#error "Socket namespace layout requires CONFIG_NET_NS"
#endif
_Static_assert(sizeof(struct ifreq) == 40, "expected native ARM64 ifreq");

enum {
	sock_from_file_arity =
		__builtin_types_compatible_p(__typeof__(&sock_from_file),
					     struct socket *(*)(struct file *)) ?
			1 :
		__builtin_types_compatible_p(__typeof__(&sock_from_file),
					     struct socket *(*)(struct file *,
								int *)) ?
			2 :
			0
};
_Static_assert(sock_from_file_arity, "unexpected sock_from_file ABI");
_Static_assert(sizeof(unsigned long) == 8, "expected ARM64 kernel");
const unsigned long vpnhide_ifconf_offsets[]
	__attribute__((section(".vpnhide_offsets"))) = {
		__builtin_offsetof(struct net, dev_base_head),
		__builtin_offsetof(struct net_device, dev_list),
		__builtin_offsetof(struct net_device, ip_ptr),
		__builtin_offsetof(struct in_device, ifa_list),
		__builtin_offsetof(struct in_ifaddr, ifa_next),
		__builtin_offsetof(struct in_ifaddr, ifa_label),
		__builtin_offsetof(struct socket, sk),
		__builtin_offsetof(struct sock_common, skc_net),
		sock_from_file_arity,
	};
