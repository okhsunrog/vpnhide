package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.checks.CheckOutput
import dev.okhsunrog.vpnhide.checks.CheckStatus
import dev.okhsunrog.vpnhide.checks.checkGetifaddrs
import dev.okhsunrog.vpnhide.checks.checkIoctlSiocgifconf
import dev.okhsunrog.vpnhide.checks.checkIoctlSiocgifflags
import dev.okhsunrog.vpnhide.checks.checkIoctlSiocgifmtu
import dev.okhsunrog.vpnhide.checks.checkNetlinkGetlink
import dev.okhsunrog.vpnhide.checks.checkNetlinkGetroute
import dev.okhsunrog.vpnhide.checks.checkProcNetDev
import dev.okhsunrog.vpnhide.checks.checkProcNetIfInet6
import dev.okhsunrog.vpnhide.checks.checkProcNetIpv6Route
import dev.okhsunrog.vpnhide.checks.checkProcNetRoute
import dev.okhsunrog.vpnhide.checks.checkSysClassNet
import dev.okhsunrog.vpnhide.generated.HookIds

/**
 * One native (UniFFI / kmod-or-zygisk-covered) detection probe. [id] is a
 * stable, non-localized key used for logging on the Dashboard protection
 * path; [labelRes] is the user-facing localized name shown on the
 * Diagnostics screen; [run] executes the probe.
 */
internal data class NativeCheckSpec(
    val id: String,
    val labelRes: Int,
    val expectedHooks: Set<HookIds.Hook> = emptySet(),
    val run: () -> CheckOutput,
)

/**
 * Map a native probe status to the tri-state the UI uses: PASS → true (leak
 * blocked), FAIL → false (leak detected), NETWORK_BLOCKED → null (probe
 * couldn't run, e.g. ECONNREFUSED from `socket()` when the app has no network
 * permission). Single source so the Diagnostics list and the Dashboard summary
 * read the status the same way.
 */
internal fun CheckStatus.toPassed(): Boolean? =
    when (this) {
        CheckStatus.PASS -> true
        CheckStatus.FAIL -> false
        CheckStatus.NETWORK_BLOCKED -> null
    }

/**
 * The native probe suite, in display order. Run once by [runCoreChecks]; the
 * Diagnostics screen lists the results and the Dashboard summary rolls them up
 * via [toNativeResult] — no second copy of this list or second execution.
 */
internal val NATIVE_CHECKS: List<NativeCheckSpec> =
    listOf(
        NativeCheckSpec(
            id = "ioctl_flags",
            labelRes = R.string.check_ioctl_flags,
            expectedHooks = setOf(HookIds.Hook.DEV_IOCTL, HookIds.Hook.ZYGISK_IOCTL),
        ) { checkIoctlSiocgifflags() },
        NativeCheckSpec(
            id = "ioctl_mtu",
            labelRes = R.string.check_ioctl_mtu,
            expectedHooks = setOf(HookIds.Hook.DEV_IOCTL, HookIds.Hook.ZYGISK_IOCTL),
        ) { checkIoctlSiocgifmtu() },
        NativeCheckSpec(
            id = "ioctl_conf",
            labelRes = R.string.check_ioctl_conf,
            expectedHooks = setOf(HookIds.Hook.SOCK_IOCTL, HookIds.Hook.ZYGISK_IOCTL),
        ) { checkIoctlSiocgifconf() },
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
        ) { checkGetifaddrs() },
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
        ) { checkNetlinkGetlink() },
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
        ) { checkNetlinkGetroute() },
        NativeCheckSpec(
            id = "proc_route",
            labelRes = R.string.check_proc_route,
            expectedHooks = setOf(HookIds.Hook.FIB_ROUTE_SEQ_SHOW, HookIds.Hook.ZYGISK_OPENAT),
        ) { checkProcNetRoute() },
        NativeCheckSpec(
            id = "proc_ipv6_route",
            labelRes = R.string.check_proc_ipv6_route,
            expectedHooks = setOf(HookIds.Hook.IPV6_ROUTE_SEQ_SHOW, HookIds.Hook.ZYGISK_OPENAT),
        ) { checkProcNetIpv6Route() },
        NativeCheckSpec(
            id = "proc_if_inet6",
            labelRes = R.string.check_proc_if_inet6,
            expectedHooks = setOf(HookIds.Hook.INET6_FILL_IFADDR, HookIds.Hook.ZYGISK_OPENAT),
        ) { checkProcNetIfInet6() },
        NativeCheckSpec(
            id = "proc_dev",
            labelRes = R.string.check_proc_dev,
            expectedHooks = setOf(HookIds.Hook.ZYGISK_OPENAT),
        ) { checkProcNetDev() },
        NativeCheckSpec("sys_class_net", R.string.check_sys_class_net) { checkSysClassNet() },
    )
