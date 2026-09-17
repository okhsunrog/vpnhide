package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.AppVpnContext
import dev.okhsunrog.vpnhide.diagnostics.AppVpnStateSnapshot
import dev.okhsunrog.vpnhide.diagnostics.CORE_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.CheckingWhat
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticDomain
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticInputs
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunIo
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EXTRA_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.JavaCheckSpec
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_EXTRA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RoutingKnowledge
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.Situation
import dev.okhsunrog.vpnhide.diagnostics.Staleness
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.gateProjection
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/**
 * The domain wired like production, driven through its observation flows: what
 * the pure tests cannot pin, because the bug this guards lived in the wiring.
 */
class DiagnosticDomainTest {
    @Test
    fun `a tunnel re-established while the app was away is confirmed once on return, without an edge to detect`() =
        fixture { f ->
            // Nothing covers the first key either, so the owner starts the first suite itself.
            f.routing.value = known(SESSION_A, generation = 1)
            val measured = f.domain.presentation.first { it.activeRunId == null && it.measurement != null }
            assertEquals(SESSION_A.identity, measured.measurement?.context?.routing)
            assertFalse(measured.confirmationPending)
            assertEquals(1, f.suites.get())

            // Foreground return: the resume re-read is in flight, so the current value is
            // null. Nothing is owed without a current key, and the old fact is kept.
            f.routing.value = verifying(previous = SESSION_A, generation = 2)
            val verifying = f.domain.presentation.first { it.routing is RoutingKnowledge.Verifying }
            assertFalse(verifying.confirmationPending)
            assertEquals(1, f.suites.get())

            // The re-read publishes the re-established tunnel: a new routing identity,
            // which the retained measurement does not cover. One confirmation is owed,
            // worded as the run it becomes, and runs without anyone comparing samples.
            f.routing.value = known(SESSION_B, generation = 2)
            val confirmed =
                f.domain.presentation.first {
                    it.activeRunId == null && it.measurement?.context?.routing == SESSION_B.identity
                }
            assertFalse(confirmed.confirmationPending)
            assertEquals(RunOutcome.Completed, confirmed.lastAttempt?.outcome)
            assertEquals(2, f.suites.get())
            // The stale measurement was never worded as a result to re-check by hand.
            assertTrue(f.situationsSeen().none { it is Situation.Measured && it.staleness == Staleness.Changed })
        }

    @Test
    fun `the same session read again reveals no new key and reruns nothing`() =
        fixture { f ->
            // Nothing covers the first key either, so the owner starts the first suite itself.
            f.routing.value = known(SESSION_A, generation = 1)
            f.domain.presentation.first { it.activeRunId == null && it.measurement != null }

            f.routing.value = verifying(previous = SESSION_A, generation = 2)
            f.domain.presentation.first { it.routing is RoutingKnowledge.Verifying }
            f.routing.value = known(SESSION_A, generation = 2)
            f.domain.presentation.first { it.routing is RoutingKnowledge.Known }
            delay(SETTLE_MS)
            assertFalse(f.domain.presentation.value.confirmationPending)
            assertEquals(1, f.suites.get())
            // The startup intent is not a rerun either.
            assertEquals(1L, f.domain.run()?.id)
        }

    @Test
    fun `an explicit retry is always a new run and the presentation shows it as explicit`() =
        fixture { f ->
            // Nothing covers the first key either, so the owner starts the first suite itself.
            f.routing.value = known(SESSION_A, generation = 1)
            f.domain.presentation.first { it.activeRunId == null && it.measurement != null }

            f.domain.retry()
            val running = f.domain.presentation.first { it.activeRunId != null }
            assertEquals(false, running.activeRunAutomatic)
            f.domain.presentation.first { it.activeRunId == null && it.lastAttempt?.id == 2L }
            assertEquals(2, f.suites.get())
            assertTrue(
                f.situationsSeen().any {
                    it is Situation.Checking && it.what == CheckingWhat.Suite &&
                        it.reason == ReadReason.Explicit
                },
            )
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

    /** The domain on real dispatchers with the observations as plain state flows and a fake probe helper. */
    private class Fixture {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val routing = MutableStateFlow(ObservationState<AppVpnStateSnapshot>())
        val snapshots =
            MutableStateFlow<RootSnapshot?>(RootSnapshot(mapOf("current_boot_id" to "boot-1"), observationId = 1, generation = 1))
        val config = MutableStateFlow(ConfigCoordinatorView())
        val inputs = MutableStateFlow<DiagnosticInputs?>(DiagnosticInputs(SELF_PACKAGE, selfNeedsRestart = false))
        val suites = AtomicInteger()
        val situations = mutableListOf<Situation>()
        val domain =
            DiagnosticDomain(
                scope = scope,
                io = FakeIo(),
                routing = routing,
                snapshots = snapshots,
                config = config,
                inputs = inputs,
                processIdentity = "pid:1;uid:10",
                wait = { },
            )

        init {
            scope.launch { domain.situation.collect { synchronized(situations) { situations += it } } }
        }

        /** A consistent copy: the collector keeps appending on its own dispatcher. */
        fun situationsSeen(): List<Situation> = synchronized(situations) { situations.toList() }

        /** Observes exactly what production observes, from the same flows; probes hide everything. */
        private inner class FakeIo : DiagnosticRunIo {
            override suspend fun observe(
                ticket: EffectTicket,
                stage: DiagnosticStage,
            ): DiagnosticContextObservation {
                val state = routing.value
                return buildDiagnosticContextObservation(
                    selfNeedsRestart = false,
                    appVpn = AppVpnContext(state.gateProjection(), state.lastGood?.value?.identity),
                    snapshot = snapshots.value,
                    config = config.value.confirmed,
                    selfPackage = SELF_PACKAGE,
                    processIdentity = "pid:1;uid:10",
                    now = 0,
                )
            }

            override suspend fun probe(
                ticket: EffectTicket,
                stage: DiagnosticStage,
                plan: List<ProbePlanEntry>,
            ): CheckResults =
                when (stage) {
                    DiagnosticStage.Core -> {
                        suites.incrementAndGet()
                        CheckResults(
                            native = NATIVE_CHECKS.map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, "root: tun0", id = it.id) },
                            nativeExtra = NATIVE_EXTRA_CHECKS.results(),
                            coreJava = CORE_JAVA_CHECKS.results(),
                        )
                    }

                    else -> {
                        CheckResults(native = emptyList(), extraJava = EXTRA_JAVA_CHECKS.results())
                    }
                }
        }
    }

    private companion object {
        const val SELF_PACKAGE = "dev.okhsunrog.vpnhide"
        const val SETTLE_MS = 300L
        val SESSION_A = AppVpnStateSnapshot(DiagnosticGate.ROUTED, "framework:101:tun0", listOf("tun0"))
        val SESSION_B = AppVpnStateSnapshot(DiagnosticGate.ROUTED, "framework:102:tun0", listOf("tun0"))

        fun request(
            id: Long,
            generation: Long,
        ) = ObservationRequest(id, generation, startedAt = 0, reason = ReadReason.Background)

        fun known(
            snapshot: AppVpnStateSnapshot,
            generation: Long,
        ) = ObservationState(
            lastGood = ObservedValue(snapshot, request(generation, generation), finishedAt = 0),
            generation = generation,
            attempted = true,
            attemptedGeneration = generation,
        )

        /** A re-read in flight after an invalidation: the last fact is retained, the current value is null. */
        fun verifying(
            previous: AppVpnStateSnapshot,
            generation: Long,
        ) = ObservationState(
            lastGood = ObservedValue(previous, request(generation - 1, generation - 1), finishedAt = 0),
            active = request(generation, generation),
            generation = generation,
            stale = StaleMark(ReadReason.Background, since = 0),
            attempted = true,
            attemptedGeneration = generation - 1,
        )

        fun List<JavaCheckSpec>.results(): List<CheckResult> = map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, id = it.id) }
    }
}
