package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.NativeTargetCapacityWarning
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
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigCoordinatorTest {
    @Test
    fun `healing interrupted capture propagates debug even during a settings only write`() =
        coordinatorTest(manageLogging = true) {
            io.config = CanonicalConfig(debug = true, debugSwitch = false)
            coordinator.initialize()
            val caller =
                async {
                    coordinator.submit(
                        CanonicalMutation(
                            listOf(CanonicalEdit.Toggle(CanonicalToggle.Filesystem, true)),
                            activation = CanonicalActivation(native = false),
                        ),
                    )
                }
            val write = io.executions.receive()
            assertFalse(write.candidate.debug)
            write.finish(PhaseOutcome.Confirmed)
            val activation = io.executions.receive()
            assertEquals(ConfigPhase.Native, activation.phase)
            activation.finish(PhaseOutcome.Confirmed)
            assertTrue(caller.await().succeeded)
        }

    @Test
    fun `overlapping captures preserve user choice and release only the last logging owner`() =
        coordinatorTest(manageLogging = true) {
            coordinator.initialize()
            val capture = async { coordinator.submit(logging(CaptureLoggingEvent.Acquire(1))) }
            val enable = io.executions.receive()
            assertTrue(enable.candidate.debug)
            assertFalse(enable.candidate.debugSwitch)
            enable.finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            assertTrue(capture.await().succeeded)
            assertTrue(coordinator.submit(logging(CaptureLoggingEvent.Acquire(2))).succeeded)
            assertTrue(coordinator.submit(toggle(CanonicalToggle.DebugSwitch, false)).succeeded)
            assertTrue(coordinator.submit(logging(CaptureLoggingEvent.Release(1))).succeeded)
            assertTrue(requireNotNull(coordinator.view.value.confirmed).debug)
            assertEquals(1, coordinator.view.value.activeCaptures)
            val release = async { coordinator.submit(logging(CaptureLoggingEvent.Release(2))) }
            val disable = io.executions.receive()
            assertFalse(disable.candidate.debug)
            disable.finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            assertTrue(release.await().succeeded)
            assertEquals(0, coordinator.view.value.activeCaptures)
        }

    @Test
    fun `release during pause is retained and reconciled after manual recovery`() =
        coordinatorTest(manageLogging = true) {
            coordinator.initialize()
            val capture = async { coordinator.submit(logging(CaptureLoggingEvent.Acquire(1))) }
            val write = io.executions.receive()
            io.config = write.candidate
            write.finish(PhaseOutcome.Unknown)
            repeat(2) { io.recoveries.receive().complete(ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)) }
            assertFalse(capture.await().succeeded)
            assertFalse(coordinator.submit(logging(CaptureLoggingEvent.Release(1))).succeeded)
            assertEquals(0, coordinator.view.value.activeCaptures)
            coordinator.retry()
            io.recoveries.receive().complete(
                ConfigPhaseEvidence(PhaseOutcome.Confirmed, RootCanonicalRead.Available(requireNotNull(io.config))),
            )
            val repair = io.executions.receive()
            assertEquals(ConfigPhase.Persist, repair.phase)
            assertFalse(repair.candidate.debug)
            repair.finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            val view = coordinator.view.first { it.mode == ConfigCoordinatorMode.Open && it.operations.isEmpty() }
            assertFalse(requireNotNull(view.confirmed).debug)
            assertEquals(PhaseOutcome.Confirmed, view.recoveredResult?.phases?.get(ConfigPhase.Persist))
            assertEquals(PhaseOutcome.NotAttempted, view.recoveredResult?.phases?.get(ConfigPhase.Native))
        }

    @Test
    fun `reset publishes missing and subsequent edits cannot recreate deleted config`() =
        coordinatorTest {
            coordinator.initialize()
            val reset = async { coordinator.submit(resetMutation()) }
            val cleanup = io.executions.receive()
            assertEquals(ConfigPhase.Cleanup, cleanup.phase)
            io.config = null
            cleanup.finish(PhaseOutcome.Confirmed)
            assertTrue(reset.await().succeeded)
            assertEquals(ConfigCoordinatorMode.Missing, coordinator.view.value.mode)
            assertEquals(null, coordinator.view.value.confirmed)
            assertFalse(coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)).succeeded)
            assertTrue(io.executions.tryReceive().isFailure)
        }

    @Test
    fun `reset validates deletion and protects drafts and active capture`() =
        coordinatorTest(manageLogging = true) {
            coordinator.initialize()
            val reset = async { coordinator.submit(resetMutation()) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            assertEquals(
                PhaseOutcome.FailedKnown,
                reset
                    .await()
                    .operation
                    ?.phases
                    ?.get(ConfigPhase.Cleanup),
            )
            coordinator.draftChanged(1, setOf(ConfigField(listOf("apps", "com.example.app", "java"))))
            assertEquals(TransitionFailure.UiEditConflict, coordinator.submit(resetMutation()).operation?.failure)
            coordinator.draftChanged(1, emptySet())
            val capture = async { coordinator.submit(logging(CaptureLoggingEvent.Acquire(1))) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            capture.await()
            assertEquals("capture_active", coordinator.submit(resetMutation()).output)
            assertTrue(io.executions.tryReceive().isFailure)
        }

    private fun logging(event: CaptureLoggingEvent) = CanonicalMutation(emptyList(), source = OperationSource.System, captureEvent = event)

    private fun resetMutation() =
        CanonicalMutation(
            emptyList(),
            removesCanonical = true,
            protectAllDrafts = true,
            coupledCommands = listOf("reset-config"),
            activation = CanonicalActivation(native = false),
        )

    @Test
    fun `manual activation readback publishes warning without rewriting historical unknown result`() =
        coordinatorTest {
            coordinator.initialize()
            val caller = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Unknown)
            repeat(2) { io.recoveries.receive().complete(ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)) }
            val historical = caller.await()
            assertEquals(TransitionFailure.ApplicationUnknown, historical.operation?.failure)
            coordinator.retry()
            val warning = NativeTargetCapacityWarning(10, 8, 2)
            io.recoveries.receive().complete(
                ConfigPhaseEvidence(PhaseOutcome.Confirmed, RootCanonicalRead.Available(requireNotNull(io.config)), warning),
            )
            coordinator.view.first { it.mode == ConfigCoordinatorMode.Open }
            assertEquals(warning, coordinator.view.value.lastNativeCapacity)
            assertEquals(
                null,
                coordinator.view.value.lastResult
                    ?.failure,
            )
            assertEquals("", historical.output)
            assertEquals(TransitionFailure.ApplicationUnknown, historical.operation?.failure)
        }

    @Test
    fun `initialization retries once and explicit retry retains preexisting drafts`() =
        coordinatorTest {
            io.initializationResult = ConfigInitialization(ConfigCoordinatorMode.Unavailable)
            coordinator.draftChanged(1, setOf(ConfigField(listOf("debugSwitch"))))
            assertEquals(ConfigCoordinatorMode.Unavailable, coordinator.initialize())
            assertEquals(ConfigCoordinatorMode.Unavailable, coordinator.initialize())
            assertEquals(2, io.initializations)
            io.initializationResult = null
            coordinator.retry()
            coordinator.view.first { it.mode == ConfigCoordinatorMode.Open }
            assertEquals(3, io.initializations)
            val result = coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true, source = OperationSource.Bridge))
            assertEquals(TransitionFailure.UiEditConflict, result.operation?.failure)
        }

    @Test
    fun `known persistence failure removes optimistic state without invoking activation`() =
        coordinatorTest {
            coordinator.initialize()
            val caller = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)) }
            val write = io.executions.receive()
            assertEquals(true, coordinator.view.value.pending[CanonicalToggle.DebugSwitch])
            write.finish(PhaseOutcome.FailedKnown)
            assertEquals(TransitionFailure.ExecutionFailed, caller.await().operation?.failure)
            assertFalse(requireNotNull(coordinator.view.value.confirmed).debugSwitch)
            assertTrue(
                coordinator.view.value.pending
                    .isEmpty(),
            )
            assertTrue(io.executions.tryReceive().isFailure)
        }

    @Test
    fun `secret failure is separate from confirmed config and never enters observable state`() =
        coordinatorTest {
            coordinator.initialize()
            val mutation =
                CanonicalMutation(
                    listOf(CanonicalEdit.Toggle(CanonicalToggle.RememberSuperkey, true)),
                    coupledCommands = listOf("private-test-secret"),
                    activation = CanonicalActivation(ports = true),
                )
            val caller = async { coordinator.submit(mutation) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            val secret = io.executions.receive()
            assertEquals(ConfigPhase.Secret, secret.phase)
            secret.finish(PhaseOutcome.FailedKnown)
            val result = caller.await()
            assertEquals(PhaseOutcome.Confirmed, result.operation?.phases?.get(ConfigPhase.Persist))
            assertEquals(PhaseOutcome.FailedKnown, result.operation?.phases?.get(ConfigPhase.Secret))
            assertEquals(PhaseOutcome.NotAttempted, result.operation?.phases?.get(ConfigPhase.Native))
            assertTrue(requireNotNull(coordinator.view.value.confirmed).settings.rememberSuperkey)
            assertFalse(
                coordinator.view.value
                    .toString()
                    .contains("private-test-secret"),
            )
            assertFalse(result.toString().contains("private-test-secret"))
        }

    @Test
    fun `concurrent initialization joins and later call returns without restarting`() =
        coordinatorTest {
            val gate = CompletableDeferred<Unit>()
            io.initializationGate = gate
            val first = async { coordinator.initialize() }
            io.initializationStarted.receive()
            val second = async { coordinator.initialize() }
            gate.complete(Unit)
            assertEquals(ConfigCoordinatorMode.Open, first.await())
            assertEquals(ConfigCoordinatorMode.Open, second.await())
            assertEquals(ConfigCoordinatorMode.Open, coordinator.initialize())
            assertEquals(1, io.initializations)
        }

    @Test
    fun `queued patches read fresh config and publish persistence before activation completes`() =
        coordinatorTest {
            coordinator.initialize()
            val first = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)) }
            val persist = io.executions.receive()
            assertEquals(mapOf(CanonicalToggle.DebugSwitch to true), coordinator.view.value.pending)
            assertFalse(requireNotNull(coordinator.view.value.confirmed).debugSwitch)
            val second = async { coordinator.submit(toggle(CanonicalToggle.AutoHideName, true)) }
            coordinator.view.first { it.operations.size == 2 }
            persist.finish(PhaseOutcome.Confirmed)
            val native = io.executions.receive()
            assertEquals(ConfigPhase.Native, native.phase)
            assertTrue(requireNotNull(coordinator.view.value.confirmed).debugSwitch)
            assertFalse(first.isCompleted)
            native.finish(PhaseOutcome.Confirmed)
            assertTrue(first.await().succeeded)
            val next = io.executions.receive()
            assertTrue(next.candidate.debugSwitch)
            assertTrue(next.candidate.settings.autoHideVpnName)
            next.finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            assertTrue(second.await().succeeded)
            assertEquals(2, io.reads)
            assertTrue(
                coordinator.view.value.pending
                    .isEmpty(),
            )
        }

    @Test
    fun `caller cancellation leaves accepted write owned by coordinator`() =
        coordinatorTest {
            coordinator.initialize()
            val caller = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)) }
            val write = io.executions.receive()
            caller.cancel()
            write.finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            val done = coordinator.view.first { it.lastResult != null && it.operations.isEmpty() }
            assertTrue(requireNotNull(done.confirmed).debugSwitch)
            assertEquals(null, done.lastResult?.failure)
        }

    @Test
    fun `failed activation retains persisted value and independent phase result`() =
        coordinatorTest {
            coordinator.initialize()
            val caller = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true, ports = true)) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.FailedKnown)
            val result = caller.await()
            assertFalse(result.succeeded)
            assertTrue(requireNotNull(coordinator.view.value.confirmed).debugSwitch)
            assertEquals(
                mapOf(
                    ConfigPhase.Persist to PhaseOutcome.Confirmed,
                    ConfigPhase.Native to PhaseOutcome.FailedKnown,
                    ConfigPhase.Ports to PhaseOutcome.NotAttempted,
                ),
                result.operation?.phases,
            )
            assertTrue(io.executions.tryReceive().isFailure)
        }

    @Test
    fun `unknown checks exactly twice pauses queued handles and manual recovery never replays`() =
        coordinatorTest {
            coordinator.initialize()
            val first = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)) }
            val write = io.executions.receive()
            val queued = async { coordinator.submit(toggle(CanonicalToggle.AutoHideName, true)) }
            coordinator.view.first { it.operations.size == 2 }
            write.finish(PhaseOutcome.Unknown)
            repeat(2) { io.recoveries.receive().complete(ConfigPhaseEvidence(PhaseOutcome.Unknown, RootCanonicalRead.Unavailable)) }
            val historical = first.await()
            assertEquals(TransitionFailure.ApplicationUnknown, historical.operation?.failure)
            assertEquals(TransitionFailure.MutationPaused, queued.await().operation?.failure)
            assertEquals(ConfigCoordinatorMode.Paused, coordinator.view.value.mode)
            assertFalse(coordinator.submit(toggle(CanonicalToggle.DebugSwitch, false)).succeeded)
            assertTrue(io.recoveries.tryReceive().isFailure)
            assertTrue(io.executions.tryReceive().isFailure)
            coordinator.retry()
            io.config = write.candidate
            io.recoveries.receive().complete(ConfigPhaseEvidence(PhaseOutcome.Confirmed, RootCanonicalRead.Available(write.candidate)))
            coordinator.view.first { it.mode == ConfigCoordinatorMode.Open }
            assertEquals(TransitionFailure.ApplicationUnknown, historical.operation?.failure)
            assertEquals(
                PhaseOutcome.NotAttempted,
                coordinator.view.value.lastResult
                    ?.phases
                    ?.get(ConfigPhase.Native),
            )
            assertTrue(io.executions.tryReceive().isFailure)
        }

    @Test
    fun `draft registered before initialization rejects same value bridge intent without writing`() =
        coordinatorTest {
            coordinator.draftChanged(7, setOf(ConfigField(listOf("debugSwitch"))))
            coordinator.initialize()
            val result = coordinator.submit(toggle(CanonicalToggle.DebugSwitch, false, source = OperationSource.Bridge))
            assertEquals(TransitionFailure.UiEditConflict, result.operation?.failure)
            assertTrue(io.executions.tryReceive().isFailure)
        }

    @Test
    fun `fresh transform participates in conflict check and late draft reports actual persistence`() =
        coordinatorTest {
            coordinator.initialize()
            coordinator.draftChanged(7, setOf(ConfigField(listOf("settings", "autoHideVpnName"))))
            val transform =
                CanonicalMutation(emptyList(), source = OperationSource.System, transform = {
                    it.copy(settings = it.settings.copy(autoHideVpnName = true))
                })
            assertEquals(TransitionFailure.UiEditConflict, coordinator.submit(transform).operation?.failure)
            coordinator.draftChanged(7, emptySet())
            val caller = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true, source = OperationSource.Bridge)) }
            val write = io.executions.receive()
            coordinator.draftChanged(8, setOf(ConfigField(listOf("debugSwitch"))))
            write.finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            val result = requireNotNull(caller.await().operation)
            assertEquals(TransitionFailure.UiEditConflict, result.failure)
            assertTrue(result.draftPending)
            assertEquals(PhaseOutcome.Confirmed, result.phases[ConfigPhase.Persist])
        }

    @Test
    fun `no op finishes without effects and refresh never blocks next write`() =
        coordinatorTest {
            coordinator.initialize()
            assertTrue(coordinator.submit(toggle(CanonicalToggle.DebugSwitch, false)).succeeded)
            assertTrue(io.executions.tryReceive().isFailure)
            val first = async { coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            first.await()
            refreshStarted.receive()
            val next = async { coordinator.submit(toggle(CanonicalToggle.AutoHideName, true)) }
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            io.executions.receive().finish(PhaseOutcome.Confirmed)
            assertTrue(next.await().succeeded)
            assertFalse(refreshRelease.isCompleted)
        }

    @Test
    fun `missing bootstrap can fail known after readback without holding lane forever`() =
        coordinatorTest {
            io.config = null
            coordinator.initialize()
            assertFalse(coordinator.submit(toggle(CanonicalToggle.DebugSwitch, true)).succeeded)
            val caller = async { coordinator.submit(CanonicalMutation(emptyList(), bootstrap = true)) }
            io.executions.receive().finish(PhaseOutcome.Unknown)
            io.recoveries.receive().complete(ConfigPhaseEvidence(PhaseOutcome.FailedKnown, RootCanonicalRead.Missing))
            assertFalse(caller.await().succeeded)
            assertEquals(ConfigCoordinatorMode.Missing, coordinator.view.value.mode)
            assertEquals(null, coordinator.view.value.confirmed)
        }
}

private fun toggle(
    field: CanonicalToggle,
    enabled: Boolean,
    ports: Boolean = false,
    source: OperationSource = OperationSource.Ui,
) = CanonicalMutation(listOf(CanonicalEdit.Toggle(field, enabled)), source = source, activation = CanonicalActivation(ports = ports))

private fun coordinatorTest(
    manageLogging: Boolean = false,
    block: suspend CoordinatorFixture.() -> Unit,
) = runBlocking {
    withTimeout(10_000) {
        val fixture = CoordinatorFixture(this, manageLogging)
        try {
            fixture.block()
        } finally {
            fixture.close()
        }
    }
}

private class CoordinatorFixture(
    callerScope: CoroutineScope,
    manageLogging: Boolean,
) : CoroutineScope by callerScope {
    private val owner = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val io = FakeConfigIo()
    val refreshStarted = Channel<Unit>(Channel.UNLIMITED)
    val refreshRelease = CompletableDeferred<Unit>()
    val coordinator =
        ConfigCoordinator(io, owner, manageLogging = manageLogging, refresh = {
            refreshStarted.send(Unit)
            refreshRelease.await()
        })

    fun close() {
        owner.cancel()
    }
}

private class FakeConfigIo : ConfigCoordinatorIo {
    @Volatile var config: CanonicalConfig? = CanonicalConfig()
    var reads = 0
    var initializations = 0
    var initializationGate: CompletableDeferred<Unit>? = null
    var initializationResult: ConfigInitialization? = null
    val initializationStarted = Channel<Unit>(Channel.UNLIMITED)
    val executions = Channel<FakeExecution>(Channel.UNLIMITED)
    val recoveries = Channel<CompletableDeferred<ConfigPhaseEvidence>>(Channel.UNLIMITED)

    override suspend fun initialize(): ConfigInitialization {
        initializations += 1
        initializationStarted.send(Unit)
        initializationGate?.await()
        initializationResult?.let { return it }
        return ConfigInitialization(if (config == null) ConfigCoordinatorMode.Missing else ConfigCoordinatorMode.Open, config)
    }

    override suspend fun read(): RootCanonicalRead {
        reads += 1
        return config?.let(RootCanonicalRead::Available) ?: RootCanonicalRead.Missing
    }

    override suspend fun execute(
        ticket: EffectTicket,
        phase: ConfigPhase,
        base: CanonicalConfig?,
        candidate: CanonicalConfig,
        command: String,
    ): ConfigPhaseEvidence {
        check(command.isNotEmpty())
        val request = FakeExecution(phase, candidate)
        executions.send(request)
        val outcome = request.result.await()
        if (phase == ConfigPhase.Persist && outcome == PhaseOutcome.Confirmed) config = candidate
        return ConfigPhaseEvidence(outcome, config?.let(RootCanonicalRead::Available) ?: RootCanonicalRead.Missing)
    }

    override suspend fun recover(
        operationId: Long,
        phase: ConfigPhase,
    ): ConfigPhaseEvidence {
        val result = CompletableDeferred<ConfigPhaseEvidence>()
        recoveries.send(result)
        return result.await()
    }
}

private class FakeExecution(
    val phase: ConfigPhase,
    val candidate: CanonicalConfig,
) {
    val result = CompletableDeferred<PhaseOutcome>()

    fun finish(outcome: PhaseOutcome) {
        result.complete(outcome)
    }
}
