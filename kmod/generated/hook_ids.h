/* AUTO-GENERATED from data/hooks.toml — do not edit by hand. Regenerate with: uv run scripts/codegen-hooks.py */
#ifndef VPNHIDE_GENERATED_HOOK_IDS_H
#define VPNHIDE_GENERATED_HOOK_IDS_H

/* Global hook id space (data/hooks.toml). bit N == hook id N. */
#define VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW            0
#define VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW           1
#define VPNHIDE_HOOK_RTNL_FILL_IFINFO              2
#define VPNHIDE_HOOK_INET_FILL_IFADDR              3
#define VPNHIDE_HOOK_INET6_FILL_IFADDR             4
#define VPNHIDE_HOOK_DEV_IOCTL                     5
#define VPNHIDE_HOOK_SOCK_IOCTL                    6
#define VPNHIDE_HOOK_FIB_DUMP_INFO                 7
#define VPNHIDE_HOOK_RT6_FILL_NODE                 8
#define VPNHIDE_HOOK_FIB_NL_FILL_RULE              9
#define VPNHIDE_HOOK_LSPOSED_LINK_PROPERTIES       10
#define VPNHIDE_HOOK_LSPOSED_NETWORK_CAPABILITIES  11
#define VPNHIDE_HOOK_LSPOSED_NETWORK_INFO          12
#define VPNHIDE_HOOK_LSPOSED_NETWORK               13
#define VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_RESULT   14
#define VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_CALLBACK 15
#define VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_NETWORK  16
#define VPNHIDE_HOOK_LSPOSED_PACKAGE_VISIBILITY    17
#define VPNHIDE_HOOK_COUNT                         18

/* Hooks owned by each backend: apply `mask & own`, ignore foreign bits. */
#define VPNHIDE_KERNEL_HOOK_MASK 0x3ffu
#define VPNHIDE_ZYGISK_HOOK_MASK 0x0u
#define VPNHIDE_LSPOSED_HOOK_MASK 0x3fc00u

/* status error codes (protocol §5.1). */
#define VPNHIDE_ERR_OK                       0
#define VPNHIDE_ERR_UNSUPPORTED_KVER         1
#define VPNHIDE_ERR_CONFLICTING_BACKEND      2
#define VPNHIDE_ERR_SYMBOL_RESOLUTION_FAILED 3
#define VPNHIDE_ERR_PARTIAL_HOOKS            4

/* backend ids (protocol §4.3 `status backend <id>`). */
#define VPNHIDE_BACKEND_KMOD    0
#define VPNHIDE_BACKEND_KPM     1
#define VPNHIDE_BACKEND_ZYGISK  2
#define VPNHIDE_BACKEND_LSPOSED 3

/* Hook name for an id (labeling / debug). Inline so the header stays
   self-contained and an unused table never warns. */
static inline const char *vpnhide_hook_name(int id)
{
	switch (id) {
	case 0: return "fib_route_seq_show";
	case 1: return "ipv6_route_seq_show";
	case 2: return "rtnl_fill_ifinfo";
	case 3: return "inet_fill_ifaddr";
	case 4: return "inet6_fill_ifaddr";
	case 5: return "dev_ioctl";
	case 6: return "sock_ioctl";
	case 7: return "fib_dump_info";
	case 8: return "rt6_fill_node";
	case 9: return "fib_nl_fill_rule";
	case 10: return "lsposed_link_properties";
	case 11: return "lsposed_network_capabilities";
	case 12: return "lsposed_network_info";
	case 13: return "lsposed_network";
	case 14: return "lsposed_connectivity_result";
	case 15: return "lsposed_connectivity_callback";
	case 16: return "lsposed_connectivity_network";
	case 17: return "lsposed_package_visibility";
	default: return "?";
	}
}

#endif /* VPNHIDE_GENERATED_HOOK_IDS_H */
