package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.statistics.StatisticsRow
import dev.okhsunrog.vpnhide.statistics.StatisticsUnavailableReason
import dev.okhsunrog.vpnhide.statistics.buildStatisticsState
import dev.okhsunrog.vpnhide.statistics.extractProtocolBlock
import dev.okhsunrog.vpnhide.statistics.formatStatCount
import dev.okhsunrog.vpnhide.statistics.parseProtocolStatsBlock
import dev.okhsunrog.vpnhide.statistics.parseProtocolStatusBlock
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class StatisticsDataTest {
    @Test
    fun `extracts status and stats blocks from combined runtime state`() {
        val raw =
            """
            # vpnhide v1 readback
            vpnhide 1 status
            backend 0x3
            kver 0x0
            hooks 0x3fc00
            error 0x0
            meta version 1.2.3
            vpnhide 1 stats
            0x278b 0xb:0x7
            """.trimIndent()

        assertEquals(
            Protocol.Status(
                backend =
                    HookIds.Backend.LSPOSED.id
                        .toLong(),
                kver = 0,
                hooks = HookIds.LSPOSED_HOOK_MASK.toLong(),
                error = 0,
            ),
            parseProtocolStatusBlock(raw),
        )
        assertEquals(
            listOf(
                Protocol.StatEntry(
                    uid = 10123,
                    hookId =
                        HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES.id
                            .toLong(),
                    count = 7,
                ),
            ),
            parseProtocolStatsBlock(raw),
        )
        assertNull(extractProtocolBlock("not protocol\n", Protocol.Kind.STATS))
        assertEquals(
            "vpnhide 1 stats\n0x1 0x0:0x1",
            extractProtocolBlock("vpnhide 1 stats\n0x1 0x0:0x1", Protocol.Kind.STATS),
        )
    }

    @Test
    fun `builds backend rows with uid package names and hook registry data`() {
        val state = buildStatisticsState(statisticsFixtureSnapshot())

        val kmod = state.backends.single { it.backend == HookIds.Backend.KMOD }
        assertEquals(2, kmod.rows.size)
        assertEquals(3uL, kmod.totalCount)
        assertEquals("com.example.one", kmod.rows[0].packageNames.single())
        assertEquals(HookIds.Hook.SOCK_IOCTL, kmod.rows[0].hook)
        assertEquals(99L, kmod.rows[1].hookId)
        assertNull(kmod.rows[1].hook)

        assertFalse(state.backends.any { it.backend == HookIds.Backend.KPM })

        val lsposed = state.backends.single { it.backend == HookIds.Backend.LSPOSED }
        assertEquals("1.2.3", lsposed.metadata["version"])
        assertEquals(HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES, lsposed.rows.single().hook)
    }

    @Test
    fun `shows kpm native statistics when kmod is not active`() {
        val state =
            buildStatisticsState(
                RootSnapshot(
                    statisticsFixtureSnapshot()
                        .sections + ("kmod_state" to ""),
                ),
            )

        assertFalse(state.backends.any { it.backend == HookIds.Backend.KMOD })
        val kpm = state.backends.single { it.backend == HookIds.Backend.KPM }
        assertEquals(
            HookIds.Backend.KPM.id
                .toLong(),
            kpm.status?.backend,
        )
        assertEquals(
            "com.example.two",
            kpm.rows
                .single()
                .packageNames
                .single(),
        )
    }

    @Test
    fun `marks truncated kpm counters unavailable instead of totaling a partial prefix`() {
        val base = statisticsFixtureSnapshot()
        val state =
            buildStatisticsState(
                RootSnapshot(
                    base.sections +
                        mapOf(
                            "kmod_state" to "",
                            "kpm_state" to
                                base.sections.getValue("kpm_state") +
                                "\n# vpnhide truncated",
                        ),
                ),
            )

        val kpm = state.backends.single { it.backend == HookIds.Backend.KPM }
        assertEquals(StatisticsUnavailableReason.KpmStatsTruncated, kpm.unavailableReason)
        assertEquals(emptyList<StatisticsRow>(), kpm.rows)
        assertEquals(
            HookIds.Backend.KPM.id
                .toLong(),
            kpm.status?.backend,
        )
    }

    @Test
    fun `shows zygisk native statistics as unavailable when zygisk is active`() {
        val state =
            buildStatisticsState(
                RootSnapshot(
                    statisticsFixtureSnapshot().sections +
                        mapOf(
                            "kmod_module_dir" to "0",
                            "kpm_module_dir" to "0",
                            "zygisk_module_dir" to "1",
                            "kmod_state" to "",
                            "kpm_state" to "",
                            "proc_exists" to "0",
                            "current_boot_id" to "boot-1",
                            "zygisk_status" to "boot_id=boot-1",
                            "zygisk_prop" to "version=1.0",
                            "kpm_load_status" to "",
                        ),
                ),
            )

        assertFalse(state.backends.any { it.backend == HookIds.Backend.KMOD })
        assertFalse(state.backends.any { it.backend == HookIds.Backend.KPM })
        val zygisk = state.backends.single { it.backend == HookIds.Backend.ZYGISK }
        assertEquals(StatisticsUnavailableReason.ZygiskNativeStats, zygisk.unavailableReason)
        assertEquals(emptyList<StatisticsRow>(), zygisk.rows)
        assertEquals(2, state.activeBackendCount)
    }

    @Test
    fun `formats unsigned counters with grouping`() {
        assertEquals("0", formatStatCount(0uL))
        assertEquals("12,345", formatStatCount(12_345uL))
        assertEquals("18,446,744,073,709,551,615", formatStatCount(-1L))
    }

    private fun statisticsFixtureSnapshot(): RootSnapshot =
        RootSnapshot(
            mapOf(
                "pm_packages" to
                    "package:com.example.one uid:10123\n" +
                    "package:com.example.two uid:10234,1010234\n",
                "kmod_state" to
                    """
                    # folded kmod read
                    vpnhide 1 status
                    backend 0x0
                    kver 0x6019d
                    hooks 0x20003ff
                    error 0x0
                    vpnhide 1 stats
                    0x278b 0x6:0x2 0x63:0x1
                    """.trimIndent(),
                "kpm_state" to
                    """
                    vpnhide 1 status
                    backend 0x1
                    kver 0x6019d
                    hooks 0x20003ff
                    error 0x0
                    vpnhide 1 stats
                    0x27fa 0x0:0x5
                    """.trimIndent(),
                "lsposed_state" to
                    """
                    vpnhide 1 status
                    backend 0x3
                    kver 0x0
                    hooks 0x3fc00
                    error 0x0
                    meta version 1.2.3
                    vpnhide 1 stats
                    0x278b 0xb:0x7
                    """.trimIndent(),
            ),
        )
}
