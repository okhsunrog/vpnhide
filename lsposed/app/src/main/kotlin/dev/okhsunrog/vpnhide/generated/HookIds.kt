// AUTO-GENERATED from data/hooks.toml — do not edit by hand. Regenerate with: uv run scripts/codegen-hooks.py

package dev.okhsunrog.vpnhide.generated

import kotlinx.serialization.Serializable

/** Global hook id space (data/hooks.toml). bit N == hook id N. */
internal object HookIds {
    // @Serializable: Hook is embedded in the canonical VpnHideState (diagnostics
    // report + dashboard optional-hook set) that serializes straight to the debug
    // JSON; the enum serializes by entry name.
    @Serializable
    enum class Hook(
        val id: Int,
        val hookName: String,
        val note: String,
    ) {
        // /proc/net/route — IPv4 route lines
        FIB_ROUTE_SEQ_SHOW(0, "fib_route_seq_show", "/proc/net/route — IPv4 route lines"),

        // /proc/net/ipv6_route — IPv6 route lines
        IPV6_ROUTE_SEQ_SHOW(1, "ipv6_route_seq_show", "/proc/net/ipv6_route — IPv6 route lines"),

        // RTM_NEWLINK — getifaddrs() link enumeration
        RTNL_FILL_IFINFO(2, "rtnl_fill_ifinfo", "RTM_NEWLINK — getifaddrs() link enumeration"),

        // RTM_GETADDR — IPv4 address dump
        INET_FILL_IFADDR(3, "inet_fill_ifaddr", "RTM_GETADDR — IPv4 address dump"),

        // RTM_GETADDR — IPv6 address dump
        INET6_FILL_IFADDR(4, "inet6_fill_ifaddr", "RTM_GETADDR — IPv6 address dump"),

        // SIOCGIF* by name (per-interface ioctls)
        DEV_IOCTL(5, "dev_ioctl", "SIOCGIF* by name (per-interface ioctls)"),

        // SIOCGIFCONF — interface list ioctl
        SOCK_IOCTL(6, "sock_ioctl", "SIOCGIFCONF — interface list ioctl"),

        // RTM_GETROUTE — IPv4 route dump (issue #86)
        FIB_DUMP_INFO(7, "fib_dump_info", "RTM_GETROUTE — IPv4 route dump (issue #86)"),

        // RTM_GETROUTE — IPv6 route dump
        RT6_FILL_NODE(8, "rt6_fill_node", "RTM_GETROUTE — IPv6 route dump"),

        // RTM_GETRULE — policy routing rules
        FIB_NL_FILL_RULE(9, "fib_nl_fill_rule", "RTM_GETRULE — policy routing rules"),

        // LinkProperties parcel/result sanitization
        LSPOSED_LINK_PROPERTIES(10, "lsposed_link_properties", "LinkProperties parcel/result sanitization"),

        // NetworkCapabilities parcel/result sanitization
        LSPOSED_NETWORK_CAPABILITIES(11, "lsposed_network_capabilities", "NetworkCapabilities parcel/result sanitization"),

        // NetworkInfo parcel/result sanitization
        LSPOSED_NETWORK_INFO(12, "lsposed_network_info", "NetworkInfo parcel/result sanitization"),

        // Network handle replacement/filtering
        LSPOSED_NETWORK(13, "lsposed_network", "Network handle replacement/filtering"),

        // ConnectivityService synchronous result filtering
        LSPOSED_CONNECTIVITY_RESULT(14, "lsposed_connectivity_result", "ConnectivityService synchronous result filtering"),

        // ConnectivityService callback filtering
        LSPOSED_CONNECTIVITY_CALLBACK(15, "lsposed_connectivity_callback", "ConnectivityService callback filtering"),

        // ConnectivityService Network handle APIs
        LSPOSED_CONNECTIVITY_NETWORK(16, "lsposed_connectivity_network", "ConnectivityService Network handle APIs"),

        // PackageManager app-hiding filters
        LSPOSED_PACKAGE_VISIBILITY(17, "lsposed_package_visibility", "PackageManager app-hiding filters"),

        // libc ioctl() SIOCGIF* interface probes
        ZYGISK_IOCTL(18, "zygisk_ioctl", "libc ioctl() SIOCGIF* interface probes"),

        // libc getifaddrs() interface enumeration
        ZYGISK_GETIFADDRS(19, "zygisk_getifaddrs", "libc getifaddrs() interface enumeration"),

        // openat() filtering for /proc/net routes and sockets
        ZYGISK_OPENAT(20, "zygisk_openat", "openat() filtering for /proc/net routes and sockets"),

        // recvmsg() netlink dump filtering
        ZYGISK_RECVMSG(21, "zygisk_recvmsg", "recvmsg() netlink dump filtering"),

        // recv() netlink dump filtering
        ZYGISK_RECV(22, "zygisk_recv", "recv() netlink dump filtering"),

        // recvfrom() netlink dump filtering
        ZYGISK_RECVFROM(23, "zygisk_recvfrom", "recvfrom() netlink dump filtering"),

        // __recvfrom_chk() fortified netlink dump filtering
        ZYGISK_RECVFROM_CHK(24, "zygisk_recvfrom_chk", "__recvfrom_chk() fortified netlink dump filtering"),

        // SO_BINDTODEVICE / SO_BINDTOIFINDEX pre-mutation denial
        SOCKET_BIND_INTERFACE(25, "socket_bind_interface", "SO_BINDTODEVICE / SO_BINDTOIFINDEX pre-mutation denial"),

        // libc setsockopt() best-effort socket-interface bind denial
        ZYGISK_SETSOCKOPT(26, "zygisk_setsockopt", "libc setsockopt() best-effort socket-interface bind denial"),

        // Optional sysfs/proc-sys VPN interface path concealment
        FILESYSTEM_IFACE_PATHS(27, "filesystem_iface_paths", "Optional sysfs/proc-sys VPN interface path concealment"),
    }

    // Hooks owned by each backend: apply `mask and own`.
    const val KERNEL_HOOK_MASK = 0x20003ff
    const val KMOD_HOOK_MASK = 0x8000000
    const val KPM_HOOK_MASK = 0x8000000
    const val BUILTIN_HOOK_MASK = 0x8000000
    const val ZYGISK_HOOK_MASK = 0xdfc0000
    const val LSPOSED_HOOK_MASK = 0x3fc00

    /** status error codes (protocol §5.1). */
    enum class StatusError(
        val code: Int,
    ) {
        // healthy; every requested, owned hook installed
        OK(0),

        // no offset table for the running kernel — refused, no hooks
        UNSUPPORTED_KVER(1),

        // the other kernel backend (.ko<->KPM) is loaded — refused (docs/storage.md §4.3)
        CONFLICTING_BACKEND(2),

        // a required kallsyms symbol was missing — refused
        SYMBOL_RESOLUTION_FAILED(3),

        // installed, but some owned hooks did not resolve (see the hooks mask)
        PARTIAL_HOOKS(4),
    }

    /** backend ids (protocol §4.3 `status backend <id>`). */
    enum class Backend(
        val id: Int,
    ) {
        // .ko kretprobe backend
        KMOD(0),

        // KernelPatch Module backend
        KPM(1),

        // Zygisk libc-hook backend
        ZYGISK(2),

        // LSPosed Java-hook backend (system_server)
        LSPOSED(3),

        // in-tree, compile-time (built-in) kernel backend (CONFIG_VPNHIDE=y); same kernel hooks as .ko, no kprobes
        BUILTIN(4),
    }
}
