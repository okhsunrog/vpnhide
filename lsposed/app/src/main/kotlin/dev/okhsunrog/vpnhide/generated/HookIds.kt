// AUTO-GENERATED from data/hooks.toml — do not edit by hand. Regenerate with: uv run scripts/codegen-hooks.py

package dev.okhsunrog.vpnhide.generated

/** Global hook id space (data/hooks.toml). bit N == hook id N. */
internal object HookIds {
    enum class Hook(
        val id: Int,
        val hookName: String,
    ) {
        // /proc/net/route — IPv4 route lines
        FIB_ROUTE_SEQ_SHOW(0, "fib_route_seq_show"),

        // /proc/net/ipv6_route — IPv6 route lines
        IPV6_ROUTE_SEQ_SHOW(1, "ipv6_route_seq_show"),

        // RTM_NEWLINK — getifaddrs() link enumeration
        RTNL_FILL_IFINFO(2, "rtnl_fill_ifinfo"),

        // RTM_GETADDR — IPv4 address dump
        INET_FILL_IFADDR(3, "inet_fill_ifaddr"),

        // RTM_GETADDR — IPv6 address dump
        INET6_FILL_IFADDR(4, "inet6_fill_ifaddr"),

        // SIOCGIF* by name (per-interface ioctls)
        DEV_IOCTL(5, "dev_ioctl"),

        // SIOCGIFCONF — interface list ioctl
        SOCK_IOCTL(6, "sock_ioctl"),

        // RTM_GETROUTE — IPv4 route dump (issue #86)
        FIB_DUMP_INFO(7, "fib_dump_info"),

        // RTM_GETROUTE — IPv6 route dump
        RT6_FILL_NODE(8, "rt6_fill_node"),

        // RTM_GETRULE — policy routing rules
        FIB_NL_FILL_RULE(9, "fib_nl_fill_rule"),

        // LinkProperties parcel/result sanitization
        LSPOSED_LINK_PROPERTIES(10, "lsposed_link_properties"),

        // NetworkCapabilities parcel/result sanitization
        LSPOSED_NETWORK_CAPABILITIES(11, "lsposed_network_capabilities"),

        // NetworkInfo parcel/result sanitization
        LSPOSED_NETWORK_INFO(12, "lsposed_network_info"),

        // Network handle replacement/filtering
        LSPOSED_NETWORK(13, "lsposed_network"),

        // ConnectivityService synchronous result filtering
        LSPOSED_CONNECTIVITY_RESULT(14, "lsposed_connectivity_result"),

        // ConnectivityService callback filtering
        LSPOSED_CONNECTIVITY_CALLBACK(15, "lsposed_connectivity_callback"),

        // ConnectivityService Network handle APIs
        LSPOSED_CONNECTIVITY_NETWORK(16, "lsposed_connectivity_network"),

        // PackageManager app-hiding filters
        LSPOSED_PACKAGE_VISIBILITY(17, "lsposed_package_visibility"),
    }

    // Hooks owned by each backend: apply `mask and own`.
    const val KERNEL_HOOK_MASK = 0x3ff
    const val ZYGISK_HOOK_MASK = 0x0
    const val LSPOSED_HOOK_MASK = 0x3fc00

    /** status error codes (protocol §5.1). */
    enum class StatusError(
        val code: Int,
    ) {
        // healthy; every requested, owned hook installed
        OK(0),

        // no offset table for the running kernel — refused, no hooks
        UNSUPPORTED_KVER(1),

        // the other kernel backend (.ko<->KPM) is loaded — refused (protocol §1.2)
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
    }
}
