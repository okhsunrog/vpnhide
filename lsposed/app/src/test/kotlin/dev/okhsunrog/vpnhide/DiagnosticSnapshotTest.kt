package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.AppVpnContext
import dev.okhsunrog.vpnhide.diagnostics.AppVpnStateSnapshot
import dev.okhsunrog.vpnhide.diagnostics.CORE_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticCaptureOutcome
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticDomain
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticImpactEvent
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticInputs
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticRunIo
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EXTRA_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.JavaCheckSpec
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_EXTRA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.ProbePlanEntry
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.Situation
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.configReadiness
import dev.okhsunrog.vpnhide.diagnostics.coreInputsJoined
import dev.okhsunrog.vpnhide.diagnostics.desiredDeliveryIdentity
import dev.okhsunrog.vpnhide.diagnostics.diagnosticSummary
import dev.okhsunrog.vpnhide.diagnostics.gateProjection
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class DiagnosticSnapshotTest {
    @Test
    fun `completion publishes the whole packet before resuming an immediate capture waiter`() =
        runTest {
            val fixture = fixture()
            val capture =
                async(UnconfinedTestDispatcher(testScheduler)) {
                    val result = fixture.domain.captureRun(1) as DiagnosticCaptureOutcome.Ran
                    val snapshot = fixture.domain.snapshot.value
                    // This continuation runs immediately from Deferred.complete, before any queued flow collector.
                    assertEquals(result.result.attempt.id, snapshot.presentation.lastAttempt?.id)
                    assertEquals(result.result.results, snapshot.presentation.measurementResults)
                    assertEquals(snapshot.presentation, fixture.domain.presentation.value)
                    assertEquals(snapshot.situation, fixture.domain.situation.value)
                    assertTrue(snapshot.presentation.currentSuccess)
                }
            runCurrent()
            capture.await()
        }

    @Test
    fun `root checkpoint waits for acceptance and uses a newer observation if the requested one was superseded`() =
        runTest {
            val fixture = fixture()
            val requested = root(2)
            val waiting = async { fixture.domain.awaitSnapshot(requested) }
            runCurrent()
            assertFalse(waiting.isCompleted)
            fixture.rootState.value = observedRoot(root(3))
            runCurrent()
            val snapshot = waiting.await()
            assertEquals(3L, snapshot.rootSnapshot?.observationId)
            assertEquals(3L, diagnosticSummary(snapshot).snapshot?.rootObservationId)
            assertEquals(snapshot.revision, diagnosticSummary(snapshot).snapshot?.revision)
        }

    @Test
    fun `root checkpoint alone does not admit an older generation`() =
        runTest {
            val fixture = fixture()
            val waiting = async { fixture.domain.awaitSnapshot(root(2).copy(generation = 4)) }
            fixture.rootState.value = observedRoot(root(3).copy(generation = 3))
            runCurrent()
            assertFalse(waiting.isCompleted)
            fixture.rootState.value = observedRoot(root(4).copy(generation = 4))
            runCurrent()
            assertEquals(4L, waiting.await().rootSnapshot?.generation)
        }

    @Test
    fun `VPN change before root refresh is reflected in export without rewriting captured evidence`() =
        runTest {
            val fixture = fixture()
            val capture = async { fixture.domain.captureRun(1) as DiagnosticCaptureOutcome.Ran }
            runCurrent()
            val result = capture.await().result
            val before = fixture.domain.snapshot.value
            assertTrue(before.presentation.currentSuccess)

            fixture.routing.value = routing(DiagnosticGate.VPN_OFF, id = 2)
            fixture.rootState.value = observedRoot(root(2))
            runCurrent()
            val exported = fixture.domain.awaitSnapshot(root(2))
            assertEquals(Situation.VpnOff, exported.situation)
            assertFalse(exported.presentation.currentSuccess)
            assertEquals(result.attempt.id, exported.presentation.measurement?.runId)
            assertEquals(result.results, exported.presentation.measurementResults)
            // Saved snapshots are immutable, and reads/exports do not request another suite.
            assertTrue(before.presentation.currentSuccess)
            assertEquals(1, fixture.probes)
        }

    @Test
    fun `invalidated root retains module history but cannot certify current measurement`() =
        runTest {
            val fixture = fixture()
            val capture = async { fixture.domain.captureRun(1) }
            runCurrent()
            capture.await()
            fixture.rootState.value = fixture.rootState.value.copy(generation = 2, stale = StaleMark(ReadReason.Explicit, 0))
            runCurrent()
            val snapshot = fixture.domain.awaitSnapshot()
            assertEquals(1L, snapshot.rootSnapshot?.observationId)
            assertEquals(MeasurementApplicability.Unverified, snapshot.presentation.applicability)
            assertFalse(snapshot.presentation.currentSuccess)
            assertEquals(false, diagnosticSummary(snapshot).snapshot?.rootCurrent)
            assertEquals(1, fixture.probes)
        }

    @Test
    fun `export waits for source collectors that have not accepted already published changes`() =
        runTest {
            val fixture = fixture()
            fixture.routing.value = routing(DiagnosticGate.VPN_OFF, id = 2)
            // The source changed, but its collector is queued on StandardTestDispatcher.
            val export = async(UnconfinedTestDispatcher(testScheduler)) { fixture.domain.awaitSnapshot() }
            assertFalse(export.isCompleted)
            runCurrent()
            assertEquals(Situation.VpnOff, export.await().situation)
        }

    @Test
    fun `new tunnel session and configuration make an old measurement inapplicable`() =
        runTest {
            val fixture = fixture()
            val capture = async { fixture.domain.captureRun(1) }
            runCurrent()
            capture.await()
            fixture.routing.value = routing(DiagnosticGate.ROUTED, id = 2)
            val changed = CanonicalConfig(apps = mapOf(SELF to CanonicalApp(java = true)))
            fixture.config.value = fixture.config.value.copy(confirmed = changed, desiredRevision = 1)
            fixture.rootState.value = observedRoot(root(2, changed, revision = 1))
            runCurrent()
            val snapshot = fixture.domain.awaitSnapshot()
            assertEquals(MeasurementApplicability.Changed, snapshot.presentation.applicability)
            assertFalse(snapshot.presentation.currentSuccess)
            assertEquals(2L, diagnosticSummary(snapshot).snapshot?.routingObservationId)
            assertEquals(fixture.config.value, snapshot.sources.config)
            assertEquals(1, fixture.probes)
        }

    @Test
    fun `configuration dispatch publishes impact and interrupted run together before releasing its waiter`() =
        runTest {
            val fixture = fixture()
            fixture.probeGate = CompletableDeferred()
            val capture =
                async(UnconfinedTestDispatcher(testScheduler)) {
                    val result = fixture.domain.captureRun(1) as DiagnosticCaptureOutcome.Ran
                    val snapshot = fixture.domain.snapshot.value
                    assertEquals(RunOutcome.Interrupted, result.result.attempt.outcome)
                    assertEquals(result.result.attempt.id, snapshot.presentation.lastAttempt?.id)
                    assertEquals(1L, snapshot.sources.impact.changeEpoch)
                    assertEquals(DiagnosticEligibility.Applying, snapshot.presentation.eligibility)
                    assertFalse(snapshot.presentation.currentSuccess)
                }
            runCurrent()
            fixture.domain.configOperation(DiagnosticImpactEvent.Accepted(10, relevant = true))
            fixture.domain.configOperation(DiagnosticImpactEvent.Dispatched(10, ConfigPhase.Persist))
            runCurrent()
            capture.await()
            fixture.probeGate?.complete(Unit)
            runCurrent()
        }

    @Test
    fun `cancelling a snapshot waiter does not cancel the diagnostic run`() =
        runTest {
            val fixture = fixture()
            fixture.probeGate = CompletableDeferred()
            val capture = async { fixture.domain.captureRun(1) }
            runCurrent()
            capture.cancel()
            fixture.probeGate?.complete(Unit)
            runCurrent()
            assertEquals(
                RunOutcome.Completed,
                fixture.domain.snapshot.value.presentation.lastAttempt
                    ?.outcome,
            )
            assertNotNull(fixture.domain.snapshot.value.presentation.measurementResults)
        }

    @Test
    fun `background grace updates Situation in the same packet and an obsolete timer cannot restore old routing`() =
        runTest {
            val fixture = fixture()
            fixture.routing.value = routing(DiagnosticGate.VPN_OFF)
            runCurrent()
            fixture.routing.value =
                fixture.routing.value.copy(
                    generation = 2,
                    stale = StaleMark(ReadReason.Background, since = testScheduler.currentTime),
                )
            runCurrent()
            val before = fixture.domain.snapshot.value
            assertEquals(Situation.VpnOff, before.situation)
            advanceTimeBy(2_000)
            runCurrent()
            val after = fixture.domain.snapshot.value
            assertTrue(after.revision > before.revision)
            assertTrue(after.situation is Situation.Checking)
            assertEquals(before.presentation, after.presentation)

            // Start a fresh grace period, then settle it before its deadline.
            fixture.routing.value = routing(DiagnosticGate.VPN_OFF, id = 3)
            runCurrent()
            fixture.routing.value =
                fixture.routing.value.copy(
                    generation = 4,
                    stale = StaleMark(ReadReason.Background, since = testScheduler.currentTime),
                )
            runCurrent()
            advanceTimeBy(100)
            fixture.routing.value = routing(DiagnosticGate.VPN_OFF, id = 5)
            runCurrent()
            val settled = fixture.domain.snapshot.value
            advanceTimeBy(3_000)
            runCurrent()
            assertEquals(settled, fixture.domain.snapshot.value)
        }

    @Test
    fun `dashboard facts must match both root observation ID and generation`() {
        val facts = RootProjection(2, 3, "facts")
        assertTrue(facts.belongsTo(root(2).copy(generation = 3)))
        assertFalse(facts.belongsTo(root(1).copy(generation = 3)))
        assertFalse(facts.belongsTo(root(2).copy(generation = 4)))
        assertFalse(facts.belongsTo(null))
    }

    private fun TestScope.fixture(): Fixture = Fixture(backgroundScope) { testScheduler.currentTime }

    private class Fixture(
        scope: CoroutineScope,
        clock: () -> Long,
    ) {
        val routing = MutableStateFlow(routing(DiagnosticGate.ROUTED))
        val rootState = MutableStateFlow(observedRoot(root(1)))
        val roots = ProjectedStateFlow(rootState) { it.lastGood?.value }
        val config =
            MutableStateFlow(ConfigCoordinatorView(confirmed = CanonicalConfig(), desiredRevision = 0, coreCurrent = true, coreEpoch = 1))
        var probeGate: CompletableDeferred<Unit>? = null
        var probes = 0
        val domain: DiagnosticDomain =
            DiagnosticDomain(
                scope = scope,
                io =
                    object : DiagnosticRunIo {
                        override suspend fun observe(
                            ticket: EffectTicket,
                            stage: DiagnosticStage,
                        ): DiagnosticContextObservation {
                            val impact = domain.impact
                            return buildDiagnosticContextObservation(
                                selfNeedsRestart = false,
                                appVpn =
                                    AppVpnContext(
                                        routing.value.gateProjection(),
                                        routing.value.lastGood
                                            ?.value
                                            ?.identity,
                                    ),
                                snapshot = roots.value,
                                config = config.value.confirmed,
                                selfPackage = SELF,
                                processIdentity = "pid:1;uid:10",
                                now = clock(),
                                readiness = configReadiness(impact),
                                changeEpoch = impact.changeEpoch,
                                delivery = desiredDeliveryIdentity(config.value, SELF),
                                configJoined = coreInputsJoined(roots.value, config.value),
                            )
                        }

                        override suspend fun probe(
                            ticket: EffectTicket,
                            stage: DiagnosticStage,
                            plan: List<ProbePlanEntry>,
                        ): CheckResults =
                            if (stage == DiagnosticStage.Core) {
                                probeGate?.await()
                                probes++
                                CheckResults(
                                    native =
                                        NATIVE_CHECKS.map {
                                            CheckResult(
                                                it.id,
                                                "",
                                                CheckOutcome.HiddenByBackend,
                                                "root: tun0",
                                                id = it.id,
                                            )
                                        },
                                    nativeExtra = NATIVE_EXTRA_CHECKS.results(),
                                    coreJava = CORE_JAVA_CHECKS.results(),
                                )
                            } else {
                                CheckResults(native = emptyList(), extraJava = EXTRA_JAVA_CHECKS.results())
                            }
                    },
                routing = routing,
                snapshots = rootState,
                config = config,
                inputs = MutableStateFlow(DiagnosticInputs(SELF, selfNeedsRestart = false)),
                processIdentity = "pid:1;uid:10",
                clock = clock,
                wallClock = clock,
            )
    }

    private companion object {
        const val SELF = "dev.okhsunrog.vpnhide"

        fun root(
            id: Long,
            config: CanonicalConfig = CanonicalConfig(),
            revision: Long = 0,
        ): RootSnapshot =
            RootSnapshot(
                mapOf("current_boot_id" to "boot-1"),
                observationId = id,
                generation = 1,
                core = CoreSessionUpdate(CoreStateView(1, revision, RootCanonicalRead.Available(config), emptyList()), true),
            )

        fun observedRoot(root: RootSnapshot): ObservationState<RootSnapshot> =
            ObservationState(
                lastGood = ObservedValue(root, ObservationRequest(root.observationId, root.generation, 0, ReadReason.Background), 0),
                generation = root.generation,
                attempted = true,
                attemptedGeneration = root.generation,
            )

        fun routing(
            gate: DiagnosticGate,
            id: Long = 1,
        ): ObservationState<AppVpnStateSnapshot> =
            ObservationState(
                lastGood =
                    ObservedValue(
                        AppVpnStateSnapshot(gate, "framework:$id:tun0", listOf("tun0")),
                        ObservationRequest(id, id, startedAt = 0, reason = ReadReason.Background),
                        finishedAt = 0,
                    ),
                generation = id,
                attempted = true,
                attemptedGeneration = id,
            )

        fun List<JavaCheckSpec>.results(): List<CheckResult> = map { CheckResult(it.id, "", CheckOutcome.HiddenByBackend, id = it.id) }
    }
}
