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

/**
 * One native (UniFFI / kmod-or-zygisk-covered) detection probe. [id] is a
 * stable, non-localized key used for logging on the Dashboard protection
 * path; [labelRes] is the user-facing localized name shown on the
 * Diagnostics screen; [run] executes the probe.
 */
internal data class NativeCheckSpec(
    val id: String,
    val labelRes: Int,
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
        NativeCheckSpec("ioctl_flags", R.string.check_ioctl_flags) { checkIoctlSiocgifflags() },
        NativeCheckSpec("ioctl_mtu", R.string.check_ioctl_mtu) { checkIoctlSiocgifmtu() },
        NativeCheckSpec("ioctl_conf", R.string.check_ioctl_conf) { checkIoctlSiocgifconf() },
        NativeCheckSpec("getifaddrs", R.string.check_getifaddrs) { checkGetifaddrs() },
        NativeCheckSpec("netlink_getlink", R.string.check_netlink_getlink) { checkNetlinkGetlink() },
        NativeCheckSpec("netlink_getroute", R.string.check_netlink_getroute) { checkNetlinkGetroute() },
        NativeCheckSpec("proc_route", R.string.check_proc_route) { checkProcNetRoute() },
        NativeCheckSpec("proc_ipv6_route", R.string.check_proc_ipv6_route) { checkProcNetIpv6Route() },
        NativeCheckSpec("proc_if_inet6", R.string.check_proc_if_inet6) { checkProcNetIfInet6() },
        NativeCheckSpec("proc_dev", R.string.check_proc_dev) { checkProcNetDev() },
        NativeCheckSpec("sys_class_net", R.string.check_sys_class_net) { checkSysClassNet() },
    )
