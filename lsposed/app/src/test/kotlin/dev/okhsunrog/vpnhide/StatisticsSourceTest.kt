package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.statistics.CounterTelemetry
import dev.okhsunrog.vpnhide.statistics.parseCounterState
import dev.okhsunrog.vpnhide.statistics.qualifyStatistics
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StatisticsSourceTest {
    private val source = sampleNativeStatus()
    private val counters =
        parseCounterState(
            mapOf(
                "kmod_state" to
                    "vpnhide 1 status\nbackend 0x4\nkver 0x0\nhooks 0x${KERNEL_HOOKS.toHookMask().toString(
                        16,
                    )}\nerror 0x0\nvpnhide 1 stats\n0x278b 0x0:0x5\n",
                "lsposed_state" to "vpnhide 1 status\nbackend 0x3\nkver 0x0\nhooks 0x3fc00\nerror 0x0\nvpnhide 1 stats\n0x278b 0xb:0x7\n",
            ),
        )
    private val telemetry = CounterTelemetry(counters, source, 9, 2, javaBootCurrent = true)

    @Test
    fun `current rows require accepted runtime and current Java boot evidence`() {
        val qualified = qualifyStatistics(telemetry, source)
        assertEquals(2, qualified.activeBackendCount)
        assertEquals(12uL, qualified.totalCount)
        assertEquals(9L, qualified.rootObservationId)
        assertFalse(
            qualifyStatistics(telemetry.copy(javaBootCurrent = false), source)
                .backends
                .single {
                    it.backend ==
                        HookIds.Backend.LSPOSED
                }.current,
        )
    }

    @Test
    fun `disconnect epoch config and connection changes withdraw current marks without rewriting historical counters`() {
        for (changed in listOf(
            source.copy(current = false),
            source.copy(epoch = 8),
            source.copy(configRevision = 1),
            source.copy(connectionGeneration = 2),
        )) {
            val qualified = qualifyStatistics(telemetry, changed)
            assertEquals(0, qualified.activeBackendCount)
            assertTrue(qualified.backends.none { it.backend == HookIds.Backend.BUILTIN })
            assertEquals(12uL, telemetry.counters.totalCount)
            assertEquals(7uL, qualified.backends.single().totalCount)
        }
    }

    @Test
    fun `same revision new delivery cannot qualify counters from an earlier repair`() {
        val model = requireNotNull(source.model)
        val changed = source.copy(model = model.copy(delivery = model.delivery?.copy(operation = 2)))
        assertEquals(source.configRevision, changed.configRevision)
        assertEquals(0, qualifyStatistics(telemetry, changed).activeBackendCount)
        val changedRuntime = source.copy(model = model.copy(runtime = model.runtime?.copy(installedHooks = 0)))
        assertEquals(0, qualifyStatistics(telemetry, changedRuntime).activeBackendCount)
    }

    @Test
    fun `selected Zygisk with no process counters never becomes globally active`() {
        val zygisk = sampleNativeStatus(selected = NativeBackendId.Zygisk)
        val result = qualifyStatistics(telemetry.copy(nativeSource = zygisk), zygisk)
        val native = result.backends.single { it.backend == HookIds.Backend.ZYGISK }
        assertFalse(native.current)
        assertTrue(native.rows.isEmpty())
    }
}
