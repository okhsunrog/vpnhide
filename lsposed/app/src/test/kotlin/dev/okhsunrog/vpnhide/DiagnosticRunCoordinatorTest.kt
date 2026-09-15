package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CORE_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAdmission
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunCoordinator
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunHandle
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunIo
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EXTRA_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.JavaCheckSpec
import dev.okhsunrog.vpnhide.diagnostics.MeasurementContext
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_EXTRA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.diagnosticRequest
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

class DiagnosticRunCoordinatorTest {
    @Test
    fun `retry after a failed attempt allocates a new run and never resolves with the old failure`() =
        fixture { f ->
            val first = f.accept(diagnosticRequest())
            f.observations
                .receive()
                .result
                .completeExceptionally(IllegalStateException("root dropped"))
            val failed = first.await().attempt
            assertEquals(RunOutcome.NotStarted, failed.outcome)
            assertEquals(TransitionFailure.ReadFailed, failed.failure)

            val second = f.accept(diagnosticRequest())
            assertEquals(2L, second.id)
            f.eligible()
            f.core()
            f.slow()
            f.eligible()
            val completed = second.await()
            assertEquals(RunOutcome.Completed, completed.attempt.outcome)
            assertEquals(2L, completed.attempt.id)
            assertEquals(EXTRA_JAVA_CHECKS.size, completed.results?.extraJava?.size)
        }

    @Test
    fun `a cancelled waiter neither cancels nor restarts the shared run`() =
        fixture { f ->
            val handle = f.accept(diagnosticRequest())
            val waiter = f.scope.async { handle.await() }
            f.eligible()
            waiter.cancel()
            f.core()
            f.slow()
            f.eligible()
            val view = f.owner.view.first { it.core.lastAttempt != null }
            assertEquals(RunOutcome.Completed, view.core.lastAttempt?.outcome)
            assertEquals(1L, view.core.lastComplete?.runId)
            assertNotNull(view.attemptResults[1L])
            assertTrue(f.observations.tryReceive().isFailure)
        }

    @Test
    fun `slow phase failure retains partial evidence beside the previous complete measurement`() =
        fixture { f ->
            f.complete(f.accept(diagnosticRequest()))
            val second = f.accept(diagnosticRequest())
            f.eligible()
            f.core()
            val partial = f.owner.view.first { it.activeResults != null }
            assertEquals(NATIVE_CHECKS.size, partial.activeResults?.native?.size)
            f.probes
                .receive()
                .result
                .completeExceptionally(IllegalStateException("callback crashed"))
            val result = second.await()
            assertEquals(RunOutcome.Failed, result.attempt.outcome)
            assertEquals(TransitionFailure.ExecutionFailed, result.attempt.failure)
            assertEquals(
                coreIds(),
                result.attempt.measurement
                    ?.outcomes
                    ?.keys,
            )
            assertEquals(NATIVE_CHECKS.size, result.results?.native?.size)
            assertTrue(result.results?.extraJava.isNullOrEmpty())
            val view = f.owner.view.value
            assertEquals(1L, view.core.lastComplete?.runId)
            assertEquals(setOf(1L, 2L), view.attemptResults.keys)
            assertNull(view.activeResults)
        }

    @Test
    fun `blocked eligibility finishes without probes and keeps the automatic intent`() =
        fixture { f ->
            val first = f.accept(diagnosticRequest(automatic = true))
            f.observations
                .receive()
                .result
                .complete(DiagnosticContextObservation(DiagnosticEligibility.VpnOff, null))
            val blocked = first.await().attempt
            assertEquals(RunOutcome.NotStarted, blocked.outcome)
            assertEquals(DiagnosticEligibility.VpnOff, blocked.eligibility)
            assertNull(blocked.failure)
            assertTrue(f.probes.tryReceive().isFailure)

            val second = f.accept(diagnosticRequest(automatic = true))
            assertEquals(2L, second.id)
            f.complete(second)
            assertTrue(f.owner.request(diagnosticRequest(automatic = true)) is DiagnosticAdmission.Ignored)
        }

    @Test
    fun `ensure joins the active run and later reads the finished attempt without a retry`() =
        fixture { f ->
            val explicit = f.accept(diagnosticRequest())
            val joined = requireNotNull(f.owner.ensure(diagnosticRequest()))
            assertEquals(explicit.id, joined.id)
            f.eligible()
            f.core()
            f.slow()
            f.eligible()
            assertEquals(RunOutcome.Completed, joined.await().attempt.outcome)
            val later = requireNotNull(f.owner.ensure(diagnosticRequest()))
            assertEquals(explicit.id, later.id)
            assertEquals(RunOutcome.Completed, later.await().attempt.outcome)
            assertTrue(f.observations.tryReceive().isFailure)
            assertNull(f.owner.view.value.core.active)
        }

    @Test
    fun `dependent observation invalidated by the eligibility read cannot form a retry cycle`() =
        fixture { f ->
            val loads = AtomicInteger()
            lateinit var observer: ObservationCoordinator<RunOutcome>
            observer =
                ObservationCoordinator(f.scope, load = {
                    loads.incrementAndGet()
                    requireNotNull(f.owner.ensure(diagnosticRequest())).await().attempt.outcome
                })
            val read = f.scope.async { observer.read() }
            val observation = f.observations.receive()
            // The routing refresh behind the eligibility read invalidates the root-dependent Dashboard.
            observer.invalidate()
            observation.result.complete(DiagnosticContextObservation(DiagnosticEligibility.VpnOff, null))
            assertEquals(RunOutcome.NotStarted, read.await())
            assertEquals(2, loads.get())
            assertTrue(f.observations.tryReceive().isFailure)
            assertNull(observer.state.value.active)
        }

    @Test
    fun `cancel with an unfinished helper quarantines probes until the helper returns`() =
        fixture { f ->
            val handle = f.accept(diagnosticRequest())
            f.eligible()
            val stale = f.probes.receive()
            f.owner.cancel(handle.id)
            assertEquals(
                DiagnosticStage.Draining,
                f.owner.view.value.core.active
                    ?.stage,
            )
            f.drainDeadlines.receive().complete(Unit)
            val interrupted = handle.await().attempt
            assertEquals(RunOutcome.Interrupted, interrupted.outcome)
            assertEquals(TransitionFailure.Cancelled, interrupted.failure)
            assertTrue(f.owner.view.value.core.quarantined)
            assertEquals(
                DiagnosticAdmission.Rejected(TransitionFailure.ResourceUnavailable),
                f.owner.request(diagnosticRequest()),
            )

            stale.result.complete(coreResults())
            val recovered = f.owner.view.first { !it.core.quarantined }
            assertNull(recovered.core.active)
            assertEquals(
                1,
                recovered.core.lastAttempt
                    ?.id
                    ?.toInt(),
            )
            val next = f.accept(diagnosticRequest())
            assertEquals(2L, next.id)
            assertEquals(
                DiagnosticStage.Checking,
                f.owner.view.value.core.active
                    ?.stage,
            )
        }

    @Test
    fun `changed end context interrupts the run and retains its evidence`() =
        fixture { f ->
            val handle = f.accept(diagnosticRequest())
            f.eligible(routing = "vpn=tun0;self=ROUTED")
            f.core()
            f.slow()
            f.eligible(routing = "vpn=tun1;self=ROUTED")
            val result = handle.await()
            assertEquals(RunOutcome.Interrupted, result.attempt.outcome)
            assertEquals(TransitionFailure.ContextChanged, result.attempt.failure)
            assertTrue(result.attempt.measurement?.interrupted == true)
            assertEquals(
                allIds(),
                result.attempt.measurement
                    ?.outcomes
                    ?.keys,
            )
            assertNull(f.owner.view.value.core.lastComplete)
            assertNotNull(result.results)
        }

    @Test
    fun `run deadline while checking finishes without probes and ignores the late observation`() =
        fixture { f ->
            val handle = f.accept(diagnosticRequest())
            val late = f.observations.receive()
            f.runDeadlines.receive().complete(Unit)
            val attempt = handle.await().attempt
            assertEquals(RunOutcome.NotStarted, attempt.outcome)
            assertEquals(TransitionFailure.DeadlineExceeded, attempt.failure)
            late.result.complete(DiagnosticContextObservation(DiagnosticEligibility.Eligible, context()))
            assertTrue(f.probes.tryReceive().isFailure)
            assertNull(f.owner.view.value.core.active)
        }

    private fun fixture(block: suspend (Fixture) -> Unit) =
        runBlocking {
            val fixture = Fixture()
            try {
                withTimeout(10_000) { block(fixture) }
            } finally {
                fixture.scope.cancel()
            }
        }

    private class Observation(
        val stage: DiagnosticStage,
        val result: CompletableDeferred<DiagnosticContextObservation> = CompletableDeferred(),
    )

    private class Probe(
        val stage: DiagnosticStage,
        val plan: List<ProbePlanEntry>,
        val result: CompletableDeferred<CheckResults> = CompletableDeferred(),
    )

    private class Fixture : CoroutineScope by CoroutineScope(SupervisorJob() + Dispatchers.Default) {
        val scope: CoroutineScope get() = this
        val observations = Channel<Observation>(Channel.UNLIMITED)
        val probes = Channel<Probe>(Channel.UNLIMITED)
        val runDeadlines = Channel<CompletableDeferred<Unit>>(Channel.UNLIMITED)
        val drainDeadlines = Channel<CompletableDeferred<Unit>>(Channel.UNLIMITED)
        val owner =
            DiagnosticRunCoordinator(
                scope = scope,
                io =
                    object : DiagnosticRunIo {
                        override suspend fun observe(
                            ticket: EffectTicket,
                            stage: DiagnosticStage,
                        ) = Observation(stage).also { observations.send(it) }.result.await()

                        override suspend fun probe(
                            ticket: EffectTicket,
                            stage: DiagnosticStage,
                            plan: List<ProbePlanEntry>,
                        ) = Probe(stage, plan).also { probes.send(it) }.result.await()
                    },
                runDeadline = { CompletableDeferred<Unit>().also { runDeadlines.send(it) }.await() },
                drainDeadline = { CompletableDeferred<Unit>().also { drainDeadlines.send(it) }.await() },
            )

        fun accept(request: dev.okhsunrog.vpnhide.diagnostics.DiagnosticRequest): DiagnosticRunHandle =
            (owner.request(request) as DiagnosticAdmission.Accepted).handle

        suspend fun eligible(routing: String = "vpn=tun0;self=ROUTED") {
            observations.receive().result.complete(DiagnosticContextObservation(DiagnosticEligibility.Eligible, context(routing)))
        }

        suspend fun core() {
            val probe = probes.receive()
            assertEquals(DiagnosticStage.Core, probe.stage)
            probe.result.complete(coreResults())
        }

        suspend fun slow() {
            val probe = probes.receive()
            assertEquals(DiagnosticStage.Slow, probe.stage)
            probe.result.complete(CheckResults(native = emptyList(), extraJava = EXTRA_JAVA_CHECKS.results()))
        }

        suspend fun complete(handle: DiagnosticRunHandle) {
            eligible()
            core()
            slow()
            eligible()
            assertEquals(RunOutcome.Completed, handle.await().attempt.outcome)
        }
    }
}

private fun context(routing: String = "vpn=tun0;self=ROUTED") =
    MeasurementContext("pid:1;uid:10", "self=null", routing, "backend=Kmod", 0, 1, 10)

private fun List<JavaCheckSpec>.results(): List<CheckResult> = map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, id = it.id) }

private fun coreResults(): CheckResults =
    CheckResults(
        native = NATIVE_CHECKS.map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, "root: tun0", id = it.id) },
        nativeExtra = NATIVE_EXTRA_CHECKS.results(),
        coreJava = CORE_JAVA_CHECKS.results(),
    )

private fun coreIds(): Set<String> =
    (
        NATIVE_CHECKS.map {
            it.id
        } + NATIVE_EXTRA_CHECKS.map { it.id } + CORE_JAVA_CHECKS.map { it.id }
    ).toSet()

private fun allIds(): Set<String> = coreIds() + EXTRA_JAVA_CHECKS.map { it.id }
