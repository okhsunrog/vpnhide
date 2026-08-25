package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.AppProbeStats
import dev.okhsunrog.vpnhide.diagnostics.DetectionMethod
import dev.okhsunrog.vpnhide.diagnostics.MethodSurface
import dev.okhsunrog.vpnhide.diagnostics.buildAppProbeStats
import dev.okhsunrog.vpnhide.diagnostics.diffCapture
import dev.okhsunrog.vpnhide.diagnostics.resolveAppSummary
import dev.okhsunrog.vpnhide.diagnostics.snapshotCounters
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.picker.AppSummary
import dev.okhsunrog.vpnhide.statistics.BackendStatistics
import dev.okhsunrog.vpnhide.statistics.StatisticsRow
import dev.okhsunrog.vpnhide.statistics.StatisticsState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ProbeStatsTest {
    private fun row(
        uid: Long,
        hook: HookIds.Hook,
        count: Long,
        pkg: String = "pkg$uid",
    ) = StatisticsRow(uid = uid, packageNames = listOf(pkg), hookId = hook.id.toLong(), hook = hook, count = count)

    private fun backend(
        id: HookIds.Backend,
        rows: List<StatisticsRow>,
    ) = BackendStatistics(backend = id, status = null, rows = rows)

    @Test
    fun `every hook maps to a method`() {
        // Exhaustive-when is enforced at compile time; this also pins surfaces.
        HookIds.Hook.entries.forEach { DetectionMethod.of(it) }
        assertEquals(MethodSurface.Java, DetectionMethod.of(HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES).surface)
        assertEquals(MethodSurface.Native, DetectionMethod.of(HookIds.Hook.DEV_IOCTL).surface)
        assertEquals(MethodSurface.Native, DetectionMethod.of(HookIds.Hook.ZYGISK_GETIFADDRS).surface)
        assertEquals(MethodSurface.Package, DetectionMethod.of(HookIds.Hook.LSPOSED_PACKAGE_VISIBILITY).surface)
        // Hooks fold into a shared method.
        assertEquals(DetectionMethod.Routes, DetectionMethod.of(HookIds.Hook.FIB_ROUTE_SEQ_SHOW))
        assertEquals(DetectionMethod.Routes, DetectionMethod.of(HookIds.Hook.FIB_DUMP_INFO))
        assertEquals(DetectionMethod.SocketBinding, DetectionMethod.of(HookIds.Hook.SOCKET_BIND_INTERFACE))
        assertEquals(DetectionMethod.SocketBinding, DetectionMethod.of(HookIds.Hook.ZYGISK_SETSOCKOPT))
    }

    @Test
    fun `aggregates per app across backends, folds hooks into methods, sorts by total`() {
        val state =
            StatisticsState(
                backends =
                    listOf(
                        backend(
                            HookIds.Backend.LSPOSED,
                            listOf(
                                row(10100, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 5),
                                row(10100, HookIds.Hook.LSPOSED_LINK_PROPERTIES, 3),
                            ),
                        ),
                        backend(
                            HookIds.Backend.KMOD,
                            listOf(
                                row(10100, HookIds.Hook.DEV_IOCTL, 2),
                                row(10200, HookIds.Hook.FIB_ROUTE_SEQ_SHOW, 7),
                                row(10200, HookIds.Hook.FIB_DUMP_INFO, 1),
                            ),
                        ),
                    ),
            )

        val apps = buildAppProbeStats(state)

        assertEquals(listOf(10100L, 10200L), apps.map { it.uid }) // total 10 before total 8
        val first = apps[0]
        assertEquals(10uL, first.total)
        assertEquals(
            mapOf(
                DetectionMethod.NetworkCapabilities to 5L,
                DetectionMethod.LinkProperties to 3L,
                DetectionMethod.InterfaceIoctl to 2L,
            ),
            first.byMethod,
        )
        assertEquals(setOf(MethodSurface.Java, MethodSurface.Native), first.surfaces)

        val second = apps[1]
        assertEquals(8uL, second.total)
        // The two route hooks folded into one Routes method.
        assertEquals(mapOf(DetectionMethod.Routes to 8L), second.byMethod)
        assertEquals(setOf(MethodSurface.Native), second.surfaces)
    }

    @Test
    fun `self package is excluded from the list`() {
        val state =
            StatisticsState(
                backends =
                    listOf(
                        backend(
                            HookIds.Backend.LSPOSED,
                            listOf(
                                row(10100, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 5, pkg = "com.other"),
                                row(10999, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 99, pkg = "dev.okhsunrog.vpnhide"),
                            ),
                        ),
                    ),
            )

        val apps = buildAppProbeStats(state, selfPackage = "dev.okhsunrog.vpnhide")

        assertEquals(listOf(listOf("com.other")), apps.map { it.packageNames })
    }

    @Test
    fun `empty state yields no apps`() {
        assertEquals(emptyList<AppProbeStats>(), buildAppProbeStats(StatisticsState(backends = emptyList())))
    }

    @Test
    fun `capture diff shows only probes since the baseline`() {
        val baseline =
            snapshotCounters(
                StatisticsState(listOf(backend(HookIds.Backend.LSPOSED, listOf(row(10100, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 5))))),
            )
        val current =
            StatisticsState(
                listOf(
                    backend(
                        HookIds.Backend.LSPOSED,
                        listOf(
                            row(10100, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 8), // +3
                            row(10200, HookIds.Hook.FIB_ROUTE_SEQ_SHOW, 4), // new, +4
                        ),
                    ),
                ),
            )

        val diff = diffCapture(baseline, current)

        assertEquals(false, diff.backendReset)
        assertEquals(listOf(10200L, 10100L), diff.apps.map { it.uid }) // delta 4 before delta 3
        assertEquals(4uL, diff.apps[0].total)
        assertEquals(mapOf(DetectionMethod.Routes to 4L), diff.apps[0].byMethod)
        assertEquals(3uL, diff.apps[1].total)
        assertEquals(mapOf(DetectionMethod.NetworkCapabilities to 3L), diff.apps[1].byMethod)
    }

    @Test
    fun `capture diff flags a backend restart when a counter drops`() {
        val baseline =
            snapshotCounters(
                StatisticsState(listOf(backend(HookIds.Backend.LSPOSED, listOf(row(10100, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 5))))),
            )
        val afterReboot =
            StatisticsState(listOf(backend(HookIds.Backend.LSPOSED, listOf(row(10100, HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, 2)))))

        val diff = diffCapture(baseline, afterReboot)

        assertEquals(true, diff.backendReset)
        assertEquals(emptyList<AppProbeStats>(), diff.apps)
    }

    private fun appStats(
        vararg packages: String,
        uid: Long = 10100,
    ) = AppProbeStats(uid = uid, packageNames = packages.toList(), total = 1uL, byHook = emptyMap())

    private fun summary(pkg: String) = AppSummary(packageName = pkg, label = pkg.uppercase(), icon = null, isSystem = false)

    @Test
    fun `resolveAppSummary returns the installed match for the uid's package`() {
        val byPackage = mapOf("com.a" to summary("com.a"), "com.b" to summary("com.b"))
        assertEquals("com.a", resolveAppSummary(appStats("com.a"), byPackage)?.packageName)
    }

    @Test
    fun `resolveAppSummary picks the first matching package for a shared uid`() {
        // Only the second package of the shared UID is installed.
        val onlySecond = mapOf("com.b" to summary("com.b"))
        assertEquals("com.b", resolveAppSummary(appStats("com.a", "com.b"), onlySecond)?.packageName)
        // When several match, the first listed one wins.
        val both = mapOf("com.a" to summary("com.a"), "com.b" to summary("com.b"))
        assertEquals("com.a", resolveAppSummary(appStats("com.a", "com.b"), both)?.packageName)
    }

    @Test
    fun `resolveAppSummary is null when nothing installed matches or no packages are listed`() {
        val byPackage = mapOf("com.a" to summary("com.a"))
        assertNull(resolveAppSummary(appStats("com.unknown"), byPackage))
        assertNull(resolveAppSummary(appStats(), byPackage)) // unknown uid, no packages
    }
}
