package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.isTerminalDiagnosticState
import dev.okhsunrog.vpnhide.diagnostics.routedTransitions
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticObservationTest {
    @Test
    fun `temporary unknown readiness cannot repeatedly trigger diagnostic retries`() =
        runBlocking {
            val routed = DiagnosticGate.ROUTED
            val events = flowOf(null, routed, null, routed, routed, DiagnosticGate.VPN_OFF, null, routed, null, routed)
            assertEquals(listOf(routed, routed), events.routedTransitions().toList())
        }

    @Test
    fun `partial diagnostic results are not terminal`() {
        assertFalse(isTerminalDiagnosticState(DiagnosticsCache.State.NotRun))
        assertFalse(isTerminalDiagnosticState(DiagnosticsCache.State.Running))
        val partial = DiagnosticsCache.State.Ready(CheckResults(native = emptyList()), complete = false)
        assertFalse(isTerminalDiagnosticState(partial))
        assertTrue(isTerminalDiagnosticState(partial.copy(complete = true)))
    }
}
