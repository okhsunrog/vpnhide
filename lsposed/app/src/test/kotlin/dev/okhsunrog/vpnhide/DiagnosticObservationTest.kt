package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.awaitDiagnosticObservation
import dev.okhsunrog.vpnhide.diagnostics.isTerminalDiagnosticState
import dev.okhsunrog.vpnhide.diagnostics.routedTransitions
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

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

    @Test
    fun `observing terminal diagnostics does not trigger a retry`() =
        runBlocking {
            val terminals =
                listOf(
                    DiagnosticsCache.State.Blocked(DiagnosticGate.VPN_OFF),
                    DiagnosticsCache.State.Failed,
                    DiagnosticsCache.State.Ready(CheckResults(native = emptyList()), complete = true),
                )
            for (terminal in terminals) {
                val state = MutableStateFlow<DiagnosticsCache.State>(terminal)
                assertEquals(terminal, awaitDiagnosticObservation(state) { error("unexpected retry") })
            }
        }

    @Test
    fun `diagnostic refresh invalidating its observer cannot form a retry cycle`() =
        runBlocking {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            try {
                withTimeout(5_000) {
                    val diagnostics = MutableStateFlow<DiagnosticsCache.State>(DiagnosticsCache.State.NotRun)
                    val runs = AtomicInteger()
                    val loads = AtomicInteger()
                    lateinit var observer: ObservationCoordinator<DiagnosticsCache.State>
                    observer =
                        ObservationCoordinator(scope, load = {
                            loads.incrementAndGet()
                            awaitDiagnosticObservation(diagnostics) {
                                runs.incrementAndGet()
                                // A diagnostic gate refresh invalidates the root-dependent Dashboard.
                                observer.invalidate()
                                diagnostics.value = DiagnosticsCache.State.Blocked(DiagnosticGate.VPN_OFF)
                            }
                        })
                    assertEquals(DiagnosticsCache.State.Blocked(DiagnosticGate.VPN_OFF), observer.read())
                    assertEquals(1, runs.get())
                    assertEquals(2, loads.get())
                    assertNull(observer.state.value.active)
                }
            } finally {
                scope.cancel()
            }
        }
}
