package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.CoreSessionUpdate
import dev.okhsunrog.vpnhide.CoreStateView
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.RootCanonicalRead
import dev.okhsunrog.vpnhide.RootSnapshot
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreDiagnosticException
import uniffi.vpnhide_core_ffi.CoreDiagnosticRefusal

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class AutonomousDiagnosticsTest {
    private fun publication(sequence: Long) =
        CoreSessionUpdate(CoreStateView(7, 0, RootCanonicalRead.Missing, emptyList()), current = true, sequence = sequence)

    private fun config(sequence: Long) =
        ConfigCoordinatorView(coreCurrent = true, coreEpoch = 7, desiredRevision = 0, coreSequence = sequence)

    @Test
    fun `failed optional metadata preserves Core read while cancellation still propagates`() =
        runTest {
            val core = MutableStateFlow(publication(1))
            val facade =
                AutonomousDiagnostics(
                    backgroundScope,
                    core,
                    MutableStateFlow(ObservationState()),
                    MutableStateFlow(config(1)),
                ) { error("no run") }
            val errors = mutableListOf<String>()
            val root = readAutonomousRoot({ error("core root snapshot probe exit=-1") }, { errors += it.message.orEmpty() })
            assertNull(root)
            assertEquals(listOf("core root snapshot probe exit=-1"), errors)
            assertEquals(AutonomousAvailability.NoRun, facade.awaitSnapshot(root).availability)
            assertTrue(facade.snapshot.value.sources.core.current)
            val cancellation = kotlinx.coroutines.CancellationException("reader left")
            try {
                readAutonomousRoot({ throw cancellation }, { error("must propagate cancellation") })
                error("cancellation lost")
            } catch (actual: kotlinx.coroutines.CancellationException) {
                org.junit.Assert.assertSame(cancellation, actual)
            }
        }

    @Test
    fun `opening subscriptions reading state and changing observations never admit a suite`() =
        runTest {
            val core = MutableStateFlow(publication(1))
            val root = MutableStateFlow<ObservationState<RootSnapshot>>(ObservationState())
            val config = MutableStateFlow(config(1))
            var requests = 0
            val facade = AutonomousDiagnostics(backgroundScope, core, root, config) { requests++ }
            assertEquals(AutonomousAvailability.NoRun, facade.awaitSnapshot().availability)
            core.value = publication(2)
            config.value = config(2)
            runCurrent()
            facade.awaitSnapshot()
            advanceTimeBy(3_600_000)
            runCurrent()
            assertEquals(0, requests)
        }

    @Test
    fun `explicit retry is process owned coalesces pending taps and never publishes its reply`() =
        runTest {
            val core = MutableStateFlow(publication(1))
            val root = MutableStateFlow<ObservationState<RootSnapshot>>(ObservationState())
            val config = MutableStateFlow(config(1))
            val reply = CompletableDeferred<Unit>()
            var requests = 0
            val facade =
                AutonomousDiagnostics(backgroundScope, core, root, config) {
                    requests++
                    reply.await()
                }
            facade.retry()
            facade.retry()
            runCurrent()
            assertEquals(1, requests)
            assertTrue(facade.retryState.value.pending)
            val reader = async(start = CoroutineStart.UNDISPATCHED) { facade.awaitSnapshot(RootSnapshot(emptyMap(), observationId = 8)) }
            reader.cancel()
            assertTrue(facade.retryState.value.pending)
            reply.complete(Unit)
            runCurrent()
            assertFalse(facade.retryState.value.pending)
            assertNull(facade.snapshot.value.run)
            assertEquals(AutonomousAvailability.NoRun, facade.snapshot.value.availability)
        }

    @Test
    fun `refused retry preserves accepted evidence and never retries itself`() =
        runTest {
            val core = MutableStateFlow(publication(1))
            val root = MutableStateFlow<ObservationState<RootSnapshot>>(ObservationState())
            val config = MutableStateFlow(config(1))
            var requests = 0
            val facade =
                AutonomousDiagnostics(backgroundScope, core, root, config) {
                    requests++
                    throw CoreDiagnosticException.Refused(CoreDiagnosticRefusal.BUSY)
                }
            facade.retry()
            runCurrent()
            assertEquals(AutonomousRetryFailure.Refused(CoreDiagnosticRefusal.BUSY), facade.retryState.value.failure)
            assertTrue(facade.snapshot.value.sources.core.current)
            advanceTimeBy(60_000)
            runCurrent()
            assertEquals(1, requests)
            assertEquals(AutonomousAvailability.NoRun, facade.awaitSnapshot().availability)
        }
}
