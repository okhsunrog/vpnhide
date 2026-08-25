package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.checks.CheckStatus
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.generated.HookIds

/**
 * One native (kmod / KPM / zygisk-covered) detection probe. [id] is the stable,
 * non-localized key that matches the Rust probe registry (`run_all` in
 * lsposed/native) — the app joins probe results to these specs by id. [labelRes]
 * is the user-facing localized name shown on the Diagnostics screen;
 * [expectedHooks] are the hooks that should cover this vector.
 */
internal data class NativeCheckSpec(
    val id: String,
    val labelRes: Int,
    val expectedHooks: Set<HookIds.Hook> = emptySet(),
)

/**
 * The native probe suite, in display order. The probes themselves run in Rust
 * (one JSON call via [dev.okhsunrog.vpnhide.checks.NativeProbe]); these specs
 * carry the display label and the expected-hook coverage, joined to the probe
 * results by [NativeCheckSpec.id].
 */
internal val NATIVE_CHECKS: List<NativeCheckSpec> =
    listOf(
        NativeCheckSpec(
            id = "ioctl_flags",
            labelRes = R.string.check_ioctl_flags,
            expectedHooks = setOf(HookIds.Hook.DEV_IOCTL, HookIds.Hook.ZYGISK_IOCTL),
        ),
        NativeCheckSpec(
            id = "ioctl_mtu",
            labelRes = R.string.check_ioctl_mtu,
            expectedHooks = setOf(HookIds.Hook.DEV_IOCTL, HookIds.Hook.ZYGISK_IOCTL),
        ),
        NativeCheckSpec(
            id = "ioctl_conf",
            labelRes = R.string.check_ioctl_conf,
            expectedHooks = setOf(HookIds.Hook.SOCK_IOCTL, HookIds.Hook.ZYGISK_IOCTL),
        ),
        NativeCheckSpec(
            id = "getifaddrs",
            labelRes = R.string.check_getifaddrs,
            expectedHooks =
                setOf(
                    HookIds.Hook.RTNL_FILL_IFINFO,
                    HookIds.Hook.INET_FILL_IFADDR,
                    HookIds.Hook.INET6_FILL_IFADDR,
                    HookIds.Hook.ZYGISK_GETIFADDRS,
                ),
        ),
        NativeCheckSpec(
            // setsockopt(SO_BINDTODEVICE, tun0): the kernel backends (kmod/KPM) own
            // this vector — they deny it in sock_setsockopt / sk_setsockopt (6.1+)
            // before sk_bound_dev_if mutates. Zygisk has only an opportunistic
            // bionic-routed setsockopt hook and does not claim the vector in its
            // owned mask, so under zygisk a leak here is (correctly) unowned.
            id = "so_bindtodevice",
            labelRes = R.string.check_so_bindtodevice,
            expectedHooks = setOf(HookIds.Hook.SOCKET_BIND_INTERFACE),
        ),
        NativeCheckSpec(
            id = "netlink_getlink",
            labelRes = R.string.check_netlink_getlink,
            expectedHooks =
                setOf(
                    HookIds.Hook.RTNL_FILL_IFINFO,
                    HookIds.Hook.INET_FILL_IFADDR,
                    HookIds.Hook.INET6_FILL_IFADDR,
                    HookIds.Hook.ZYGISK_RECVMSG,
                    HookIds.Hook.ZYGISK_RECV,
                    HookIds.Hook.ZYGISK_RECVFROM,
                    HookIds.Hook.ZYGISK_RECVFROM_CHK,
                ),
        ),
        NativeCheckSpec(
            id = "netlink_getroute",
            labelRes = R.string.check_netlink_getroute,
            expectedHooks =
                setOf(
                    HookIds.Hook.FIB_DUMP_INFO,
                    HookIds.Hook.RT6_FILL_NODE,
                    HookIds.Hook.ZYGISK_RECVMSG,
                    HookIds.Hook.ZYGISK_RECV,
                    HookIds.Hook.ZYGISK_RECVFROM,
                    HookIds.Hook.ZYGISK_RECVFROM_CHK,
                ),
        ),
        NativeCheckSpec(
            // Kernel backends trim the per-UID tun policy rules via fib_nl_fill_rule;
            // zygisk parses RTM_NEWRULE in the same recv netlink filter as GETLINK/
            // GETROUTE, dropping rules that name a VPN iface or steer this uid into a
            // tun table (see filter::rule_hides_vpn).
            id = "netlink_getrule",
            labelRes = R.string.check_netlink_getrule,
            expectedHooks =
                setOf(
                    HookIds.Hook.FIB_NL_FILL_RULE,
                    HookIds.Hook.ZYGISK_RECVMSG,
                    HookIds.Hook.ZYGISK_RECV,
                    HookIds.Hook.ZYGISK_RECVFROM,
                    HookIds.Hook.ZYGISK_RECVFROM_CHK,
                ),
        ),
        NativeCheckSpec(
            id = "proc_route",
            labelRes = R.string.check_proc_route,
            expectedHooks = setOf(HookIds.Hook.FIB_ROUTE_SEQ_SHOW, HookIds.Hook.ZYGISK_OPENAT),
        ),
        NativeCheckSpec(
            id = "proc_ipv6_route",
            labelRes = R.string.check_proc_ipv6_route,
            expectedHooks = setOf(HookIds.Hook.IPV6_ROUTE_SEQ_SHOW, HookIds.Hook.ZYGISK_OPENAT),
        ),
        NativeCheckSpec(
            // No kernel seq_show hook exists for /proc/net/if_inet6 — INET6_FILL_IFADDR
            // covers the netlink RTM_GETADDR path, not this procfs read. So a kernel
            // backend does not own this vector; only the zygisk openat filter (or SELinux).
            id = "proc_if_inet6",
            labelRes = R.string.check_proc_if_inet6,
            expectedHooks = setOf(HookIds.Hook.ZYGISK_OPENAT),
        ),
        NativeCheckSpec(
            id = "proc_dev",
            labelRes = R.string.check_proc_dev,
            expectedHooks = setOf(HookIds.Hook.ZYGISK_OPENAT),
        ),
        NativeCheckSpec(
            "sys_class_net",
            R.string.check_sys_class_net,
            expectedHooks = setOf(HookIds.Hook.FILESYSTEM_IFACE_PATHS),
        ),
        NativeCheckSpec(
            "proc_sys_net",
            R.string.check_proc_sys_net,
            expectedHooks = setOf(HookIds.Hook.FILESYSTEM_IFACE_PATHS),
        ),
    )
