package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds

// Where a detection method lives — used to group methods on the per-app card and
// to explain (native syscall/libc vs Java API vs package enumeration) at a
// glance. "Native" covers both the kernel backends and Zygisk's libc hooks.
internal enum class MethodSurface { Java, Native, Package }

// User-facing detection method: a small taxonomy over the 18 raw hooks so the
// Statistics screen reads as "what the app tried" instead of kernel symbol
// names. Several hooks fold into one method (e.g. the four route hooks).
internal enum class DetectionMethod(
    val surface: MethodSurface,
    val labelRes: Int,
    // Explains what the probe is and how an app uses it to infer a VPN is up.
    val descriptionRes: Int,
) {
    Routes(MethodSurface.Native, R.string.method_routes, R.string.method_desc_routes),
    Interfaces(MethodSurface.Native, R.string.method_interfaces, R.string.method_desc_interfaces),
    InterfaceIoctl(MethodSurface.Native, R.string.method_interface_ioctl, R.string.method_desc_interface_ioctl),
    PolicyRules(MethodSurface.Native, R.string.method_policy_rules, R.string.method_desc_policy_rules),
    NetworkCapabilities(MethodSurface.Java, R.string.method_network_capabilities, R.string.method_desc_network_capabilities),
    LinkProperties(MethodSurface.Java, R.string.method_link_properties, R.string.method_desc_link_properties),
    NetworkInfo(MethodSurface.Java, R.string.method_network_info, R.string.method_desc_network_info),
    NetworkHandle(MethodSurface.Java, R.string.method_network_handle, R.string.method_desc_network_handle),
    ConnectivityService(MethodSurface.Java, R.string.method_connectivity_service, R.string.method_desc_connectivity_service),
    PackageEnumeration(MethodSurface.Package, R.string.method_package_enumeration, R.string.method_desc_package_enumeration),
    ;

    companion object {
        fun of(hook: HookIds.Hook): DetectionMethod =
            when (hook) {
                HookIds.Hook.FIB_ROUTE_SEQ_SHOW,
                HookIds.Hook.IPV6_ROUTE_SEQ_SHOW,
                HookIds.Hook.FIB_DUMP_INFO,
                HookIds.Hook.RT6_FILL_NODE,
                -> Routes

                HookIds.Hook.RTNL_FILL_IFINFO,
                HookIds.Hook.INET_FILL_IFADDR,
                HookIds.Hook.INET6_FILL_IFADDR,
                -> Interfaces

                HookIds.Hook.DEV_IOCTL,
                HookIds.Hook.SOCK_IOCTL,
                -> InterfaceIoctl

                HookIds.Hook.FIB_NL_FILL_RULE -> PolicyRules

                HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES -> NetworkCapabilities

                HookIds.Hook.LSPOSED_LINK_PROPERTIES -> LinkProperties

                HookIds.Hook.LSPOSED_NETWORK_INFO -> NetworkInfo

                HookIds.Hook.LSPOSED_NETWORK,
                HookIds.Hook.LSPOSED_CONNECTIVITY_NETWORK,
                -> NetworkHandle

                HookIds.Hook.LSPOSED_CONNECTIVITY_RESULT,
                HookIds.Hook.LSPOSED_CONNECTIVITY_CALLBACK,
                -> ConnectivityService

                HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY -> PackageEnumeration

                // Zygisk libc hooks — native-level probes from inside the app.
                HookIds.Hook.ZYGISK_IOCTL -> InterfaceIoctl

                HookIds.Hook.ZYGISK_GETIFADDRS -> Interfaces

                HookIds.Hook.ZYGISK_OPENAT -> Routes

                HookIds.Hook.ZYGISK_RECVMSG,
                HookIds.Hook.ZYGISK_RECV,
                HookIds.Hook.ZYGISK_RECVFROM,
                HookIds.Hook.ZYGISK_RECVFROM_CHK,
                -> Interfaces
            }
    }
}

// Per-app rollup of probe counts, aggregated across every active backend (the
// one native backend + Java). Counts are cumulative since each backend started.
// [byHook] is the exact per-hook breakdown the backends report (for the detail
// modal); [byMethod] folds those hooks into the user-facing taxonomy (for the
// card chips).
internal data class AppProbeStats(
    val uid: Long,
    val packageNames: List<String>,
    val total: ULong,
    val byHook: Map<HookIds.Hook, Long>,
) {
    val byMethod: Map<DetectionMethod, Long> =
        byHook.entries
            .groupBy({ DetectionMethod.of(it.key) }, { it.value })
            .mapValues { (_, counts) -> counts.sum() }

    val surfaces: Set<MethodSurface> = byMethod.keys.map { it.surface }.toSet()
}

// Collapse the per-backend / per-(uid×hook) rows into one entry per app, with a
// per-method breakdown. Hooks map to methods; an unknown hook id still counts
// toward the app total but has no method bucket. Sorted by total, descending.
//
// [selfPackage] (VPN Hide's own package) is excluded: the app's cold-start
// diagnostic check suite probes every vector against itself, which would
// otherwise dominate the list as self-noise rather than a real prober.
internal fun buildAppProbeStats(
    state: StatisticsState,
    selfPackage: String? = null,
): List<AppProbeStats> {
    class Acc {
        var total: ULong = 0uL
        var packages: List<String> = emptyList()
        val byHook = linkedMapOf<HookIds.Hook, Long>()
    }

    val byUid = linkedMapOf<Long, Acc>()
    state.backends
        .asSequence()
        .flatMap { it.rows.asSequence() }
        .forEach { row ->
            val acc = byUid.getOrPut(row.uid) { Acc() }
            acc.total += row.count.toULong()
            if (row.packageNames.isNotEmpty()) acc.packages = row.packageNames
            val hook = row.hook ?: return@forEach
            acc.byHook[hook] = (acc.byHook[hook] ?: 0L) + row.count
        }

    return byUid
        .map { (uid, acc) ->
            AppProbeStats(
                uid = uid,
                packageNames = acc.packages,
                total = acc.total,
                byHook = acc.byHook.toMap(),
            )
        }.filterNot { selfPackage != null && selfPackage in it.packageNames }
        .sortedWith(
            compareByDescending<AppProbeStats> { it.total }
                .thenBy { it.packageNames.joinToString() }
                .thenBy { it.uid },
        )
}

// ── Capture session: baseline-diff over a user-controlled window ──────────
//
// No backend reset command and no per-event timestamps — the app snapshots the
// cumulative counters at "Start capture" and shows current − baseline. Lets a
// user start a session, exercise a target app, and see exactly what it probed
// in that window, on any active backend.

/** Snapshot of cumulative counters keyed by (uid, hookId), taken at capture start. */
internal fun snapshotCounters(state: StatisticsState): Map<Pair<Long, Long>, Long> =
    state.backends
        .flatMap { it.rows }
        .associate { (it.uid to it.hookId) to it.count }

internal data class CaptureDiff(
    val apps: List<AppProbeStats>,
    // A counter dropped below its baseline — the backend restarted (reboot /
    // system_server restart / Zygisk re-inject). The caller should re-baseline.
    val backendReset: Boolean,
)

// A finished capture session, frozen at "Stop". Keeps the per-app rollup and the
// elapsed duration so the results stay on screen for review after the live
// session ends — the user clears them explicitly instead of losing them the
// instant they stop.
internal data class FrozenCapture(
    val apps: List<AppProbeStats>,
    val durationMs: Long,
)

/** Probes that happened since [baseline] was taken, as a per-app rollup. */
internal fun diffCapture(
    baseline: Map<Pair<Long, Long>, Long>,
    current: StatisticsState,
    selfPackage: String? = null,
): CaptureDiff {
    var reset = false
    val deltaRows =
        current.backends
            .flatMap { it.rows }
            .mapNotNull { row ->
                val base = baseline[row.uid to row.hookId] ?: 0L
                val delta = row.count - base
                when {
                    delta < 0L -> {
                        reset = true
                        null
                    }

                    delta == 0L -> {
                        null
                    }

                    else -> {
                        row.copy(count = delta)
                    }
                }
            }
    val synthetic =
        StatisticsState(backends = listOf(BackendStatistics(backend = HookIds.Backend.LSPOSED, status = null, rows = deltaRows)))
    return CaptureDiff(apps = buildAppProbeStats(synthetic, selfPackage), backendReset = reset)
}
