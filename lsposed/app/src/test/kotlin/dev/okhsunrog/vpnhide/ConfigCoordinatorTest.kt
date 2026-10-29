package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreCommand
import uniffi.vpnhide_core_ffi.CoreEdit
import uniffi.vpnhide_core_ffi.CoreFieldEdit

@OptIn(ExperimentalCoroutinesApi::class)
class ConfigCoordinatorTest {
    @Test
    fun `one typed batch owns the receipt and core publishes confirmed configuration`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            assertEquals(ConfigCoordinatorMode.Open, init.await())
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            val request = core.submissions.receive()
            val command = request.command as CoreCommand.Edit
            val edit = (command.edit as CoreEdit.Batch).edits.single() as CoreFieldEdit.Toggle
            assertTrue(edit.enabled)
            core.complete(request, CanonicalConfig(debugSwitch = true))
            runCurrent()
            assertTrue(saving.await().succeeded)
            assertTrue(requireNotNull(owner.view.value.confirmed).debugSwitch)
            assertTrue(
                owner.view.value.operations
                    .isEmpty(),
            )
            assertEquals(1, core.submitCount)
        }

    @Test
    fun `cancelling a screen waiter does not cancel its accepted operation`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            val request = core.submissions.receive()
            saving.cancel()
            core.complete(request, CanonicalConfig(debugSwitch = true))
            runCurrent()
            assertTrue(requireNotNull(owner.view.value.confirmed).debugSwitch)
            assertTrue(
                owner.view.value.operations
                    .isEmpty(),
            )
        }

    @Test
    fun `core logcat lease publications update app logging without a config action`() =
        runTest {
            val core = FakeSession()
            var appDebug = false
            val owner = ConfigCoordinator({ core }, backgroundScope, confirmed = { appDebug = it.debug })
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            core.publish(CanonicalConfig(debug = true), debugLease = true)
            runCurrent()
            assertTrue(appDebug)
            assertTrue(requireNotNull(owner.view.value.confirmed).debug)
            core.publish(CanonicalConfig(), debugLease = false)
            runCurrent()
            assertFalse(appDebug)
            assertEquals(0, core.submitCount)
        }

    @Test
    fun `same value bridge intent respects drafts before dispatch`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            owner.draftChanged(5, setOf(ConfigField(listOf("debugSwitch"))))
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving =
                async {
                    owner.submit(
                        CanonicalMutation(
                            listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, false)),
                            source = OperationSource.Bridge,
                        ),
                    )
                }
            runCurrent()
            assertEquals(TransitionFailure.UiEditConflict, saving.await().operation?.failure)
            assertEquals(0, core.submitCount)
        }

    @Test
    fun `late draft conflict retains actual persistence evidence`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving =
                async {
                    owner.submit(
                        CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)), source = OperationSource.Bridge),
                    )
                }
            runCurrent()
            val request = core.submissions.receive()
            owner.draftChanged(5, setOf(ConfigField(listOf("debugSwitch"))))
            runCurrent()
            core.complete(request, CanonicalConfig(debugSwitch = true))
            runCurrent()
            val result = requireNotNull(saving.await().operation)
            assertEquals(TransitionFailure.UiEditConflict, result.failure)
            assertEquals(PhaseOutcome.Confirmed, result.phases[ConfigPhase.Persist])
            assertTrue(result.draftPending)
        }

    @Test
    fun `lost submission is queried by the same id and never submitted again`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving =
                async {
                    owner.submit(
                        CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)), source = OperationSource.Bridge),
                    )
                }
            runCurrent()
            val request = core.submissions.receive()
            owner.draftChanged(5, setOf(ConfigField(listOf("debugSwitch"))))
            runCurrent()
            request.reply.complete(CoreReply.Uncertain)
            runCurrent()
            val query = core.queries.receive()
            assertEquals(request.op, query.op)
            query.reply.complete(CoreReply.Observed(core.state))
            runCurrent()
            val unresolved = requireNotNull(saving.await().operation)
            assertEquals(TransitionFailure.ApplicationUnknown, unresolved.failure)
            assertTrue(unresolved.conflicts.isNotEmpty())
            assertEquals(ConfigCoordinatorMode.Paused, owner.view.value.mode)
            owner.retry()
            runCurrent()
            val recheck = core.queries.receive()
            core.publish(CanonicalConfig(debugSwitch = true))
            recheck.reply.complete(CoreReply.Observed(core.state, receipt(request.op)))
            runCurrent()
            assertEquals(ConfigCoordinatorMode.Open, owner.view.value.mode)
            assertEquals(
                unresolved.id,
                owner.view.value.recoveredResult
                    ?.id,
            )
            assertEquals(1, core.submitCount)
        }

    @Test
    fun `uncertain secret command is recovered by its durable id without replay`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving = async { owner.submit(CanonicalMutation(emptyList(), coupledEffects = listOf(CoreRootEffect.ClearSuperkey))) }
            runCurrent()
            val request = core.submissions.receive()
            assertEquals(CoreCommand.ClearSuperkey(0u), request.command)
            request.reply.complete(CoreReply.Uncertain)
            runCurrent()
            val query = core.queries.receive()
            assertEquals(request.op, query.op)
            query.reply.complete(CoreReply.Observed(core.state))
            runCurrent()
            assertEquals(TransitionFailure.ApplicationUnknown, saving.await().operation?.failure)
            owner.retry()
            runCurrent()
            core.queries
                .receive()
                .reply
                .complete(CoreReply.Observed(core.state, receipt(request.op)))
            runCurrent()
            assertEquals(1, core.submitCount)
            assertEquals(ConfigCoordinatorMode.Open, owner.view.value.mode)
        }

    @Test
    fun `disconnect retains last good values but prevents new writes`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            core.updates.value = core.updates.value.copy(current = false)
            runCurrent()
            assertEquals(ConfigCoordinatorMode.Unavailable, owner.view.value.mode)
            assertEquals(CanonicalConfig(), owner.view.value.confirmed)
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            assertFalse(saving.await().succeeded)
            assertEquals(0, core.submitCount)
        }

    @Test
    fun `failed receipt queries keep uncertain admission paused`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            val request = core.submissions.receive()
            request.reply.complete(CoreReply.Uncertain)
            runCurrent()
            core.queries
                .receive()
                .reply
                .complete(CoreReply.Unreachable)
            runCurrent()
            assertEquals(TransitionFailure.ApplicationUnknown, saving.await().operation?.failure)
            owner.retry()
            runCurrent()
            core.queries
                .receive()
                .reply
                .complete(CoreReply.ProtocolSkew)
            runCurrent()
            assertEquals(ConfigCoordinatorMode.Paused, owner.view.value.mode)
            val refused = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, false)))) }
            runCurrent()
            assertFalse(refused.await().succeeded)
            assertEquals(1, core.submitCount)
        }

    @Test
    fun `saving a key is one durable command and displays core phase evidence`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving =
                async {
                    owner.submit(
                        CanonicalMutation(
                            listOf(CanonicalEdit.Toggle(CanonicalToggle.RememberSuperkey, true)),
                            coupledEffects = listOf(CoreRootEffect.SetSuperkey("private")),
                        ),
                    )
                }
            runCurrent()
            val request = core.submissions.receive()
            val command = request.command as CoreCommand.SetSuperkey
            assertTrue(command.remember)
            core.publish(CanonicalConfig(settings = CanonicalSettings(rememberSuperkey = true)))
            val record =
                CoreOperationRecord(
                    request.op,
                    mapOf(
                        ConfigPhase.Persist to PhaseOutcome.Confirmed,
                        ConfigPhase.Secret to PhaseOutcome.Confirmed,
                        ConfigPhase.Native to PhaseOutcome.FailedKnown,
                    ),
                    PhaseOutcome.FailedKnown,
                )
            request.reply.complete(CoreReply.Accepted(request.op, record, core.state))
            runCurrent()
            val result = saving.await()
            assertFalse(result.succeeded)
            assertEquals(PhaseOutcome.Confirmed, result.operation?.phases?.get(ConfigPhase.Secret))
            assertEquals(PhaseOutcome.Confirmed, result.operation?.phases?.get(ConfigPhase.Persist))
            assertEquals(1, core.submitCount)
        }

    @Test
    fun `accepted running receipt is queried even when its publication already reports no operation`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            val command = core.submissions.receive()
            command.reply.complete(CoreReply.Accepted(command.op, null, core.state))
            runCurrent()
            assertFalse(saving.isCompleted)
            val query = core.queries.receive()
            assertEquals(command.op, query.op)
            query.reply.complete(CoreReply.Observed(core.state, receipt(command.op)))
            runCurrent()
            assertTrue(saving.await().succeeded)
            assertEquals(ConfigCoordinatorMode.Open, owner.view.value.mode)
            assertEquals(1, core.submitCount)
        }

    @Test
    fun `explicit repair is available after restart for core held native delivery`() =
        runTest {
            val core = FakeSession()
            core.updates.value =
                CoreSessionUpdate(
                    core.state.copy(
                        mutationHeld = true,
                        heldResources = setOf(ConfigPhase.Native),
                        recovery = uniffi.vpnhide_core_ffi.CoreRecoveryAccess(null, null, null, null, null),
                    ),
                    true,
                )
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            assertEquals(ConfigCoordinatorMode.Paused, owner.view.value.mode)
            assertTrue(owner.view.value.canRepair)
            val repair = async { owner.submit(CanonicalMutation(emptyList(), forceActivation = true)) }
            runCurrent()
            val command = core.submissions.receive()
            assertEquals(CoreEdit.RepairActivation, (command.command as CoreCommand.Edit).edit)
            core.updates.value = CoreSessionUpdate(core.state.copy(mutationHeld = false, heldResources = emptySet()), true)
            command.reply.complete(CoreReply.Accepted(command.op, receipt(command.op), core.state))
            runCurrent()
            assertTrue(repair.await().succeeded)
            assertEquals(ConfigCoordinatorMode.Open, owner.view.value.mode)
        }

    @Test
    fun `published capture ownership blocks full reset before dispatch`() =
        runTest {
            val core = FakeSession()
            core.updates.value = CoreSessionUpdate(core.state.copy(activeCaptures = 1), true)
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val reset =
                async {
                    owner.submit(
                        CanonicalMutation(
                            emptyList(),
                            removesCanonical = true,
                            coupledEffects = listOf(CoreRootEffect.FullReset),
                        ),
                    )
                }
            runCurrent()
            assertEquals("capture_active", reset.await().output)
            assertEquals(0, core.submitCount)
        }

    @Test
    fun `accepted pending operation stays active until publication and receipt settle`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            val request = core.submissions.receive()
            core.updates.value = CoreSessionUpdate(core.state.copy(operationsInFlight = listOf(request.op)), true, sequence = 1)
            request.reply.complete(CoreReply.Accepted(request.op, receipt(request.op).copy(settled = null), core.state))
            runCurrent()
            assertFalse(saving.isCompleted)
            assertTrue(
                owner.view.value.operations
                    .isNotEmpty(),
            )
            assertEquals(null, owner.view.value.lastResult)
            core.updates.value = CoreSessionUpdate(core.state.copy(operationsInFlight = emptyList()), true, sequence = 2)
            runCurrent()
            val query = core.queries.receive()
            assertEquals(request.op, query.op)
            query.reply.complete(CoreReply.Observed(core.state, receipt(request.op)))
            runCurrent()
            assertTrue(saving.await().succeeded)
            assertEquals(1, core.submitCount)
        }

    @Test
    fun `storage fault after admission pauses without inventing a terminal receipt`() =
        runTest {
            val core = FakeSession()
            val owner = ConfigCoordinator({ core }, backgroundScope)
            val init = async { owner.initialize() }
            runCurrent()
            init.await()
            val saving = async { owner.submit(CanonicalMutation(listOf(CanonicalEdit.Toggle(CanonicalToggle.DebugSwitch, true)))) }
            runCurrent()
            val request = core.submissions.receive()
            core.updates.value = CoreSessionUpdate(core.state.copy(operationsInFlight = listOf(request.op)), true, sequence = 1)
            request.reply.complete(CoreReply.Accepted(request.op, receipt(request.op).copy(settled = null), core.state))
            runCurrent()
            core.updates.value = CoreSessionUpdate(core.state.copy(storageFault = "disk failed", mutationHeld = true), true, sequence = 2)
            runCurrent()
            assertEquals(TransitionFailure.ApplicationUnknown, saving.await().operation?.failure)
            assertEquals("disk failed", owner.view.value.storageFault)
            assertEquals(1, core.submitCount)
        }

    private class FakeSession : CoreSessionClient {
        override val updates = MutableStateFlow(CoreSessionUpdate(state(), current = true))
        val state: CoreStateView get() = requireNotNull(updates.value.state)
        val submissions = Channel<Submission>(Channel.UNLIMITED)
        val queries = Channel<Query>(Channel.UNLIMITED)
        var submitCount = 0

        override suspend fun read(): CoreReply = CoreReply.Observed(state)

        private var next = 1000L

        override suspend fun allocateOperationId(): Long = ++next

        override suspend fun submit(
            op: Long,
            command: CoreCommand,
        ): CoreReply {
            submitCount++
            return Submission(op, command).let {
                submissions.send(it)
                it.reply.await()
            }
        }

        override suspend fun operation(op: Long): CoreReply =
            Query(op).let {
                queries.send(it)
                it.reply.await()
            }

        fun publish(
            config: CanonicalConfig,
            debugLease: Boolean = false,
        ) {
            updates.value =
                CoreSessionUpdate(
                    state.copy(
                        configRevision = state.configRevision + 1,
                        revision = state.revision + 1,
                        canonical = RootCanonicalRead.Available(config),
                        debugLease = debugLease,
                    ),
                    current = true,
                )
        }

        fun complete(
            request: Submission,
            config: CanonicalConfig,
        ) {
            publish(config)
            request.reply.complete(CoreReply.Accepted(request.op, receipt(request.op), state))
        }
    }

    private data class Submission(
        val op: Long,
        val command: CoreCommand,
        val reply: CompletableDeferred<CoreReply> = CompletableDeferred(),
    )

    private data class Query(
        val op: Long,
        val reply: CompletableDeferred<CoreReply> = CompletableDeferred(),
    )

    private companion object {
        fun state() = CoreStateView(1, 0, RootCanonicalRead.Available(CanonicalConfig()), emptyList())

        fun receipt(op: Long) =
            CoreOperationRecord(
                op,
                mapOf(
                    ConfigPhase.Persist to PhaseOutcome.Confirmed,
                    ConfigPhase.Native to PhaseOutcome.Confirmed,
                ),
                PhaseOutcome.Confirmed,
            )
    }
}
