/* AUTO-GENERATED from data/hooks.toml — do not edit by hand. Regenerate with: uv run scripts/codegen-hooks.py */
#ifndef VPNHIDE_GENERATED_HOOK_IDS_H
#define VPNHIDE_GENERATED_HOOK_IDS_H

/* Global hook id space (data/hooks.toml). bit N == hook id N. */
enum vpnhide_hook_id {
	VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW            = 0,
	VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW           = 1,
	VPNHIDE_HOOK_RTNL_FILL_IFINFO              = 2,
	VPNHIDE_HOOK_INET_FILL_IFADDR              = 3,
	VPNHIDE_HOOK_INET6_FILL_IFADDR             = 4,
	VPNHIDE_HOOK_DEV_IOCTL                     = 5,
	VPNHIDE_HOOK_SOCK_IOCTL                    = 6,
	VPNHIDE_HOOK_FIB_DUMP_INFO                 = 7,
	VPNHIDE_HOOK_RT6_FILL_NODE                 = 8,
	VPNHIDE_HOOK_FIB_NL_FILL_RULE              = 9,
	VPNHIDE_HOOK_LSPOSED_LINK_PROPERTIES       = 10,
	VPNHIDE_HOOK_LSPOSED_NETWORK_CAPABILITIES  = 11,
	VPNHIDE_HOOK_LSPOSED_NETWORK_INFO          = 12,
	VPNHIDE_HOOK_LSPOSED_NETWORK               = 13,
	VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_RESULT   = 14,
	VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_CALLBACK = 15,
	VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_NETWORK  = 16,
	VPNHIDE_HOOK_LSPOSED_PACKAGE_VISIBILITY    = 17,
	VPNHIDE_HOOK_ZYGISK_IOCTL                  = 18,
	VPNHIDE_HOOK_ZYGISK_GETIFADDRS             = 19,
	VPNHIDE_HOOK_ZYGISK_OPENAT                 = 20,
	VPNHIDE_HOOK_ZYGISK_RECVMSG                = 21,
	VPNHIDE_HOOK_ZYGISK_RECV                   = 22,
	VPNHIDE_HOOK_ZYGISK_RECVFROM               = 23,
	VPNHIDE_HOOK_ZYGISK_RECVFROM_CHK           = 24,
	VPNHIDE_HOOK_SOCKET_BIND_INTERFACE         = 25,
	VPNHIDE_HOOK_ZYGISK_SETSOCKOPT             = 26,
	VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS        = 27,
	VPNHIDE_HOOK_COUNT                         = 28,
};

static inline unsigned int vpnhide_hook_bit(enum vpnhide_hook_id id)
{
	return 1u << (unsigned int)id;
}

/* Hooks owned by each backend: apply `mask & own`, ignore foreign bits. */
#define VPNHIDE_KERNEL_HOOK_MASK 0x20003ffu
#define VPNHIDE_KMOD_HOOK_MASK 0x8000000u
#define VPNHIDE_KPM_HOOK_MASK 0x8000000u
#define VPNHIDE_BUILTIN_HOOK_MASK 0x8000000u
#define VPNHIDE_ZYGISK_HOOK_MASK 0xdfc0000u
#define VPNHIDE_LSPOSED_HOOK_MASK 0x3fc00u

/* Dense stats slots shared by the .ko and KPM kernel hooks.
   The wire keeps global hook ids; these helpers only compact the
   backend's in-memory counters. */
#define VPNHIDE_KERNEL_HOOK_COUNT 11
static inline int vpnhide_kernel_hook_slot(enum vpnhide_hook_id id)
{
	switch (id) {
	case VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW: return 0;
	case VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW: return 1;
	case VPNHIDE_HOOK_RTNL_FILL_IFINFO: return 2;
	case VPNHIDE_HOOK_INET_FILL_IFADDR: return 3;
	case VPNHIDE_HOOK_INET6_FILL_IFADDR: return 4;
	case VPNHIDE_HOOK_DEV_IOCTL: return 5;
	case VPNHIDE_HOOK_SOCK_IOCTL: return 6;
	case VPNHIDE_HOOK_FIB_DUMP_INFO: return 7;
	case VPNHIDE_HOOK_RT6_FILL_NODE: return 8;
	case VPNHIDE_HOOK_FIB_NL_FILL_RULE: return 9;
	case VPNHIDE_HOOK_SOCKET_BIND_INTERFACE: return 10;
	default: return -1;
	}
}

static inline enum vpnhide_hook_id vpnhide_kernel_hook_id(unsigned int slot)
{
	switch (slot) {
	case 0: return VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW;
	case 1: return VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW;
	case 2: return VPNHIDE_HOOK_RTNL_FILL_IFINFO;
	case 3: return VPNHIDE_HOOK_INET_FILL_IFADDR;
	case 4: return VPNHIDE_HOOK_INET6_FILL_IFADDR;
	case 5: return VPNHIDE_HOOK_DEV_IOCTL;
	case 6: return VPNHIDE_HOOK_SOCK_IOCTL;
	case 7: return VPNHIDE_HOOK_FIB_DUMP_INFO;
	case 8: return VPNHIDE_HOOK_RT6_FILL_NODE;
	case 9: return VPNHIDE_HOOK_FIB_NL_FILL_RULE;
	case 10: return VPNHIDE_HOOK_SOCKET_BIND_INTERFACE;
	default: return VPNHIDE_HOOK_COUNT;
	}
}

/* Dense stats slots for every hook the .ko can install.
   The wire keeps global hook ids; these helpers only compact the
   backend's in-memory counters. */
#define VPNHIDE_KMOD_STATS_HOOK_COUNT 12
static inline int vpnhide_kmod_stats_hook_slot(enum vpnhide_hook_id id)
{
	switch (id) {
	case VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW: return 0;
	case VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW: return 1;
	case VPNHIDE_HOOK_RTNL_FILL_IFINFO: return 2;
	case VPNHIDE_HOOK_INET_FILL_IFADDR: return 3;
	case VPNHIDE_HOOK_INET6_FILL_IFADDR: return 4;
	case VPNHIDE_HOOK_DEV_IOCTL: return 5;
	case VPNHIDE_HOOK_SOCK_IOCTL: return 6;
	case VPNHIDE_HOOK_FIB_DUMP_INFO: return 7;
	case VPNHIDE_HOOK_RT6_FILL_NODE: return 8;
	case VPNHIDE_HOOK_FIB_NL_FILL_RULE: return 9;
	case VPNHIDE_HOOK_SOCKET_BIND_INTERFACE: return 10;
	case VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS: return 11;
	default: return -1;
	}
}

static inline enum vpnhide_hook_id vpnhide_kmod_stats_hook_id(unsigned int slot)
{
	switch (slot) {
	case 0: return VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW;
	case 1: return VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW;
	case 2: return VPNHIDE_HOOK_RTNL_FILL_IFINFO;
	case 3: return VPNHIDE_HOOK_INET_FILL_IFADDR;
	case 4: return VPNHIDE_HOOK_INET6_FILL_IFADDR;
	case 5: return VPNHIDE_HOOK_DEV_IOCTL;
	case 6: return VPNHIDE_HOOK_SOCK_IOCTL;
	case 7: return VPNHIDE_HOOK_FIB_DUMP_INFO;
	case 8: return VPNHIDE_HOOK_RT6_FILL_NODE;
	case 9: return VPNHIDE_HOOK_FIB_NL_FILL_RULE;
	case 10: return VPNHIDE_HOOK_SOCKET_BIND_INTERFACE;
	case 11: return VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS;
	default: return VPNHIDE_HOOK_COUNT;
	}
}

/* Dense stats slots for every hook the KPM can install.
   The wire keeps global hook ids; these helpers only compact the
   backend's in-memory counters. */
#define VPNHIDE_KPM_STATS_HOOK_COUNT 12
static inline int vpnhide_kpm_stats_hook_slot(enum vpnhide_hook_id id)
{
	switch (id) {
	case VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW: return 0;
	case VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW: return 1;
	case VPNHIDE_HOOK_RTNL_FILL_IFINFO: return 2;
	case VPNHIDE_HOOK_INET_FILL_IFADDR: return 3;
	case VPNHIDE_HOOK_INET6_FILL_IFADDR: return 4;
	case VPNHIDE_HOOK_DEV_IOCTL: return 5;
	case VPNHIDE_HOOK_SOCK_IOCTL: return 6;
	case VPNHIDE_HOOK_FIB_DUMP_INFO: return 7;
	case VPNHIDE_HOOK_RT6_FILL_NODE: return 8;
	case VPNHIDE_HOOK_FIB_NL_FILL_RULE: return 9;
	case VPNHIDE_HOOK_SOCKET_BIND_INTERFACE: return 10;
	case VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS: return 11;
	default: return -1;
	}
}

static inline enum vpnhide_hook_id vpnhide_kpm_stats_hook_id(unsigned int slot)
{
	switch (slot) {
	case 0: return VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW;
	case 1: return VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW;
	case 2: return VPNHIDE_HOOK_RTNL_FILL_IFINFO;
	case 3: return VPNHIDE_HOOK_INET_FILL_IFADDR;
	case 4: return VPNHIDE_HOOK_INET6_FILL_IFADDR;
	case 5: return VPNHIDE_HOOK_DEV_IOCTL;
	case 6: return VPNHIDE_HOOK_SOCK_IOCTL;
	case 7: return VPNHIDE_HOOK_FIB_DUMP_INFO;
	case 8: return VPNHIDE_HOOK_RT6_FILL_NODE;
	case 9: return VPNHIDE_HOOK_FIB_NL_FILL_RULE;
	case 10: return VPNHIDE_HOOK_SOCKET_BIND_INTERFACE;
	case 11: return VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS;
	default: return VPNHIDE_HOOK_COUNT;
	}
}

/* status error codes (protocol §5.1). */
enum vpnhide_status_error {
	VPNHIDE_ERR_OK                       = 0,
	VPNHIDE_ERR_UNSUPPORTED_KVER         = 1,
	VPNHIDE_ERR_CONFLICTING_BACKEND      = 2,
	VPNHIDE_ERR_SYMBOL_RESOLUTION_FAILED = 3,
	VPNHIDE_ERR_PARTIAL_HOOKS            = 4,
};

/* backend ids (protocol §4.3 `status backend <id>`). */
enum vpnhide_backend {
	VPNHIDE_BACKEND_KMOD    = 0,
	VPNHIDE_BACKEND_KPM     = 1,
	VPNHIDE_BACKEND_ZYGISK  = 2,
	VPNHIDE_BACKEND_LSPOSED = 3,
	VPNHIDE_BACKEND_BUILTIN = 4,
};

/* Hook name for an id (labeling / debug). Inline so the header stays
   self-contained and an unused table never warns. */
static inline const char *vpnhide_hook_name(enum vpnhide_hook_id id)
{
	switch (id) {
	case VPNHIDE_HOOK_FIB_ROUTE_SEQ_SHOW: return "fib_route_seq_show";
	case VPNHIDE_HOOK_IPV6_ROUTE_SEQ_SHOW: return "ipv6_route_seq_show";
	case VPNHIDE_HOOK_RTNL_FILL_IFINFO: return "rtnl_fill_ifinfo";
	case VPNHIDE_HOOK_INET_FILL_IFADDR: return "inet_fill_ifaddr";
	case VPNHIDE_HOOK_INET6_FILL_IFADDR: return "inet6_fill_ifaddr";
	case VPNHIDE_HOOK_DEV_IOCTL: return "dev_ioctl";
	case VPNHIDE_HOOK_SOCK_IOCTL: return "sock_ioctl";
	case VPNHIDE_HOOK_FIB_DUMP_INFO: return "fib_dump_info";
	case VPNHIDE_HOOK_RT6_FILL_NODE: return "rt6_fill_node";
	case VPNHIDE_HOOK_FIB_NL_FILL_RULE: return "fib_nl_fill_rule";
	case VPNHIDE_HOOK_LSPOSED_LINK_PROPERTIES: return "lsposed_link_properties";
	case VPNHIDE_HOOK_LSPOSED_NETWORK_CAPABILITIES: return "lsposed_network_capabilities";
	case VPNHIDE_HOOK_LSPOSED_NETWORK_INFO: return "lsposed_network_info";
	case VPNHIDE_HOOK_LSPOSED_NETWORK: return "lsposed_network";
	case VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_RESULT: return "lsposed_connectivity_result";
	case VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_CALLBACK: return "lsposed_connectivity_callback";
	case VPNHIDE_HOOK_LSPOSED_CONNECTIVITY_NETWORK: return "lsposed_connectivity_network";
	case VPNHIDE_HOOK_LSPOSED_PACKAGE_VISIBILITY: return "lsposed_package_visibility";
	case VPNHIDE_HOOK_ZYGISK_IOCTL: return "zygisk_ioctl";
	case VPNHIDE_HOOK_ZYGISK_GETIFADDRS: return "zygisk_getifaddrs";
	case VPNHIDE_HOOK_ZYGISK_OPENAT: return "zygisk_openat";
	case VPNHIDE_HOOK_ZYGISK_RECVMSG: return "zygisk_recvmsg";
	case VPNHIDE_HOOK_ZYGISK_RECV: return "zygisk_recv";
	case VPNHIDE_HOOK_ZYGISK_RECVFROM: return "zygisk_recvfrom";
	case VPNHIDE_HOOK_ZYGISK_RECVFROM_CHK: return "zygisk_recvfrom_chk";
	case VPNHIDE_HOOK_SOCKET_BIND_INTERFACE: return "socket_bind_interface";
	case VPNHIDE_HOOK_ZYGISK_SETSOCKOPT: return "zygisk_setsockopt";
	case VPNHIDE_HOOK_FILESYSTEM_IFACE_PATHS: return "filesystem_iface_paths";
	default: return "?";
	}
}

#endif /* VPNHIDE_GENERATED_HOOK_IDS_H */
