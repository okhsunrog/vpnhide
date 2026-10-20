package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreEffectOutcome
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The core-only save path: Persist is a typed Replace, Native/Ports report core's receipt. */
class CoreConfigIoTest {
    private val config = CanonicalConfig()

    private fun state(
        revision: Long,
        inFlight: List<Long> = emptyList(),
        canonical: RootCanonicalRead = RootCanonicalRead.Available(config),
    ) = CoreStateView(epoch = 7, configRevision = revision, canonical = canonical, operationsInFlight = inFlight)

    private fun record(
        op: Long,
        vararg phases: Pair<ConfigPhase, PhaseOutcome>,
        settled: PhaseOutcome? = PhaseOutcome.Confirmed,
    ) = CoreOperationRecord(op, phases.toMap(), settled)

    private class FakeCore(
        var stateReply: CoreReply,
    ) : CoreConfigClient {
        val calls = mutableListOf<String>()
        var replaceReply: (Long) -> CoreReply = { CoreReply.Unreachable }
        var repairReply: (Long) -> CoreReply = { CoreReply.Unreachable }
        var operationReply: (Long) -> CoreReply = { CoreReply.Unreachable }

        override suspend fun state(): CoreReply = stateReply.also { calls += "state" }

        override suspend fun operation(op: Long): CoreReply = operationReply(op).also { calls += "operation $op" }

        override suspend fun replace(
            op: Long,
            baseRevision: Long,
            canonicalJson: String,
        ): CoreReply {
            calls += "replace $op $baseRevision"
            assertEquals(canonicalConfigJson(parseCanonicalConfig(canonicalJson)!!), canonicalJson)
            return replaceReply(op)
        }

        override suspend fun repair(
            op: Long,
            baseRevision: Long,
        ): CoreReply = repairReply(op).also { calls += "repair $op $baseRevision" }
    }

    private val applied = mutableListOf<CoreRootEffect>()
    private var effectOutcome: (CoreRootEffect) -> CoreEffectOutcome = { CoreEffectOutcome.Ok }

    private fun io(
        core: FakeCore?,
        ids: Iterator<Long> = generateSequence(100L) { it + 1 }.iterator(),
    ) = CoreConfigIo(
        prepareCore = { core },
        rootEffects = { _, effect -> effectOutcome(effect).also { applied += effect } },
        newOperationId = { ids.next() },
    )

    private val ticket = EffectTicket(owner = 1, sequence = 1)

    @Test
    fun `no core is unavailable`() =
        runTest {
            assertEquals(ConfigCoordinatorMode.Unavailable, io(null).initialize().mode)
            assertEquals(ConfigCoordinatorMode.Unavailable, io(FakeCore(CoreReply.Unreachable)).initialize().mode)
        }

    @Test
    fun `initialize maps core state and pauses while core runs an operation`() =
        runTest {
            assertEquals(ConfigCoordinatorMode.Open, io(FakeCore(CoreReply.Observed(state(3)))).initialize().mode)
            assertEquals(
                ConfigCoordinatorMode.Missing,
                io(FakeCore(CoreReply.Observed(state(0, canonical = RootCanonicalRead.Missing)))).initialize().mode,
            )
            assertEquals(ConfigCoordinatorMode.Paused, io(FakeCore(CoreReply.Observed(state(3, listOf(9))))).initialize().mode)
        }

    @Test
    fun `persist replaces against the read revision and native ports reuse the receipt`() =
        runTest {
            val core = FakeCore(CoreReply.Observed(state(4)))
            core.replaceReply = { op ->
                CoreReply.Accepted(
                    op,
                    record(
                        op,
                        ConfigPhase.Persist to PhaseOutcome.Confirmed,
                        ConfigPhase.Native to PhaseOutcome.Confirmed,
                        ConfigPhase.Ports to PhaseOutcome.FailedKnown,
                    ),
                    state(5),
                )
            }
            val io = io(core)
            io.initialize()
            io.read()
            val persist = io.execute(ticket, ConfigPhase.Persist, config, config, emptyList())
            assertEquals(PhaseOutcome.Confirmed, persist.outcome)
            assertEquals(PhaseOutcome.Confirmed, io.execute(ticket, ConfigPhase.Native, config, config, emptyList()).outcome)
            assertEquals(PhaseOutcome.FailedKnown, io.execute(ticket, ConfigPhase.Ports, config, config, emptyList()).outcome)
            assertEquals(listOf("state", "state", "replace 100 4"), core.calls)
        }

    @Test
    fun `forced native without a current receipt asks core to repair`() =
        runTest {
            val core = FakeCore(CoreReply.Observed(state(2)))
            core.repairReply = { op -> CoreReply.Accepted(op, record(op, ConfigPhase.Native to PhaseOutcome.Confirmed), state(2)) }
            val io = io(core)
            io.initialize()
            io.read()
            assertEquals(PhaseOutcome.Confirmed, io.execute(ticket, ConfigPhase.Native, config, config, emptyList()).outcome)
            // Ports was not planned by core (nothing to apply): it settles with the operation.
            assertEquals(PhaseOutcome.Confirmed, io.execute(ticket, ConfigPhase.Ports, config, config, emptyList()).outcome)
            assertEquals(listOf("state", "state", "repair 100 2"), core.calls)
        }

    @Test
    fun `refusals and an absent core are known failures`() =
        runTest {
            val core = FakeCore(CoreReply.Observed(state(1)))
            core.replaceReply = { CoreReply.Refused("conflict", "revision") }
            val io = io(core)
            io.initialize()
            assertEquals(PhaseOutcome.FailedKnown, io.execute(ticket, ConfigPhase.Persist, config, config, emptyList()).outcome)
            core.replaceReply = { CoreReply.Unreachable }
            assertEquals(PhaseOutcome.FailedKnown, io.execute(ticket, ConfigPhase.Persist, config, config, emptyList()).outcome)
        }

    @Test
    fun `an uncertain submit is recovered by the same op id, never resubmitted`() =
        runTest {
            val core = FakeCore(CoreReply.Observed(state(1)))
            core.replaceReply = { CoreReply.Uncertain }
            val io = io(core)
            io.initialize()
            assertEquals(PhaseOutcome.Unknown, io.execute(ticket, ConfigPhase.Persist, config, config, emptyList()).outcome)

            core.operationReply =
                { op -> CoreReply.Observed(state(2), record(op, ConfigPhase.Persist to PhaseOutcome.Running, settled = null)) }
            assertEquals(PhaseOutcome.Unknown, io.recover(1, ConfigPhase.Persist).outcome)

            core.operationReply = { op -> CoreReply.Observed(state(2), record(op, ConfigPhase.Persist to PhaseOutcome.Confirmed)) }
            assertEquals(PhaseOutcome.Confirmed, io.recover(1, ConfigPhase.Persist).outcome)
            assertEquals(PhaseOutcome.Confirmed, io.execute(ticket, ConfigPhase.Native, config, config, emptyList()).outcome)
            assertEquals(listOf("state", "replace 100 1", "operation 100", "operation 100"), core.calls)
        }

    @Test
    fun `coupled phases apply typed root effects in order and recover by reapplying`() =
        runTest {
            val core = FakeCore(CoreReply.Observed(state(1)))
            val io = io(core)
            io.initialize()
            val effects = listOf(CoreRootEffect.SetSuperkey("k"), CoreRootEffect.DeleteLegacyConfig)
            assertEquals(PhaseOutcome.Confirmed, io.execute(ticket, ConfigPhase.Secret, config, config, effects).outcome)
            assertEquals(effects, applied)

            applied.clear()
            effectOutcome = { if (it == CoreRootEffect.SetSuperkey("k")) CoreEffectOutcome.FailedKnown else CoreEffectOutcome.Ok }
            assertEquals(PhaseOutcome.FailedKnown, io.execute(ticket, ConfigPhase.Secret, config, config, effects).outcome)
            assertEquals(listOf(CoreRootEffect.SetSuperkey("k")), applied)

            effectOutcome = { CoreEffectOutcome.Unknown }
            assertEquals(
                PhaseOutcome.Unknown,
                io.execute(ticket, ConfigPhase.Cleanup, config, config, listOf(CoreRootEffect.FullReset)).outcome,
            )
            effectOutcome = { CoreEffectOutcome.Ok }
            applied.clear()
            assertEquals(PhaseOutcome.Confirmed, io.recover(ticket.owner, ConfigPhase.Cleanup).outcome)
            assertEquals(listOf<CoreRootEffect>(CoreRootEffect.FullReset), applied)
            // A phase this io never ran is not recoverable.
            assertEquals(PhaseOutcome.Unknown, io.recover(99, ConfigPhase.Cleanup).outcome)
            // A coupled phase with no effect is a known failure, never a silent success.
            assertEquals(PhaseOutcome.FailedKnown, io.execute(ticket, ConfigPhase.Secret, config, config, emptyList()).outcome)
        }

    @Test
    fun `apply outcome never reports an unreached phase as success`() {
        val failed =
            record(
                1,
                ConfigPhase.Native to PhaseOutcome.FailedKnown,
                ConfigPhase.Ports to PhaseOutcome.NotAttempted,
                settled = PhaseOutcome.FailedKnown,
            )
        assertEquals(PhaseOutcome.FailedKnown, applyOutcome(failed, ConfigPhase.Ports))
        val unknown = record(1, ConfigPhase.Ports to PhaseOutcome.NotAttempted, settled = PhaseOutcome.Unknown)
        assertEquals(PhaseOutcome.Unknown, applyOutcome(unknown, ConfigPhase.Ports))
        assertEquals(
            PhaseOutcome.Unknown,
            applyOutcome(record(1, ConfigPhase.Native to PhaseOutcome.Running, settled = null), ConfigPhase.Native),
        )
    }

    @Test
    fun `operation ids are positive and i64 safe`() {
        repeat(1000) {
            val id = CoreOperationIds.next()
            assertTrue(id in 1 until Long.MAX_VALUE)
        }
    }

    // --- parseCoreReply: the core-ffi JSON contract ---------------------------------------

    private val stateJson =
        """{"epoch":77,"config_revision":1,"config_status":"readable","config":${org.json.JSONObject.quote(
            canonicalConfigJson(config),
        )},"operations_in_flight":[],"backend":"Kmod"}"""

    @Test
    fun `parses accepted replies with receipts`() {
        val reply =
            parseCoreReply(
                """{"status":"accepted","op":1001,"record":{"op":1001,"phases":[{"phase":"Persist","outcome":"Confirmed"},{"phase":"Native","outcome":"FailedKnown"}],"settled":"FailedKnown"},"state":$stateJson}""",
            ) as CoreReply.Accepted
        assertEquals(1001L, reply.op)
        assertEquals(PhaseOutcome.FailedKnown, reply.record!!.phases[ConfigPhase.Native])
        assertEquals(PhaseOutcome.FailedKnown, reply.record.settled)
        assertEquals(1L, reply.state.configRevision)
        assertTrue(reply.state.canonical is RootCanonicalRead.Available)
    }

    @Test
    fun `parses state and unknown op as observations`() {
        val observed = parseCoreReply("""{"status":"ok","state":$stateJson}""") as CoreReply.Observed
        assertEquals(77L, observed.state.epoch)
        assertNull(observed.record)
        val missing =
            parseCoreReply(
                """{"status":"unknown_op","op":5,"state":{"epoch":1,"config_revision":0,"config_status":"missing","config":null,"operations_in_flight":[3],"backend":"Unknown"}}""",
            ) as CoreReply.Observed
        assertEquals(RootCanonicalRead.Missing, missing.state.canonical)
        assertEquals(listOf(3L), missing.state.operationsInFlight)
    }

    @Test
    fun `maps refusals and delivery statuses`() {
        for (status in listOf("conflict", "id_reused", "refused_unavailable", "rejected", "invalid_input")) {
            assertEquals(status, (parseCoreReply("""{"status":"$status","detail":"x"}""") as CoreReply.Refused).reason)
        }
        val capacity =
            parseCoreReply(
                """{"status":"native_capacity_exceeded","op":3,"detail":"{\"total\":162,\"capacity\":160,\"packages\":[{\"package\":\"com.a\",\"uids\":[10100,1010100]}]}"}""",
            ) as CoreReply.Refused
        assertEquals(NATIVE_CAPACITY_EXCEEDED, capacity.reason)
        assertEquals(
            NativeCapacityRefusal(162, 160, listOf(NativeCapacityPackage("com.a", listOf(10_100, 1_010_100)))),
            parseNativeCapacityRefusal(capacity.detail),
        )
        assertEquals(null, parseNativeCapacityRefusal("not json"))
        assertEquals(CoreReply.ProtocolSkew, parseCoreReply("""{"status":"protocol_skew","detail":"v4 vs v5"}"""))
        assertEquals(CoreReply.Unreachable, parseCoreReply("""{"status":"unreachable","detail":"ENOENT"}"""))
        assertEquals(CoreReply.Uncertain, parseCoreReply("""{"status":"transport","op":1,"detail":"reset"}"""))
        assertEquals(CoreReply.Uncertain, parseCoreReply("""{"status":"unavailable","detail":"x"}"""))
    }

    @Test
    fun `malformed replies are uncertain, never accepted`() {
        assertEquals(CoreReply.Uncertain, parseCoreReply(""))
        assertEquals(CoreReply.Uncertain, parseCoreReply("{not json"))
        assertEquals(CoreReply.Uncertain, parseCoreReply("""{"status":"accepted","op":1}"""))
        assertEquals(CoreReply.Uncertain, parseCoreReply("""{"status":"ok","state":$stateJson} trailing"""))
        assertEquals(
            CoreReply.Uncertain,
            parseCoreReply(
                """{"status":"accepted","op":1,"record":{"op":1,"phases":[{"phase":"Bogus","outcome":"Confirmed"}],"settled":null},"state":$stateJson}""",
            ),
        )
    }
}
