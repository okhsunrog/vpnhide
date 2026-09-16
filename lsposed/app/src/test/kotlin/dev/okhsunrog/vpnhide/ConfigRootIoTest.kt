package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.NativeTargetCapacityWarning
import dev.okhsunrog.vpnhide.picker.parseNativeTargetCapacityWarning
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ConfigRootIoTest {
    @Test
    fun `first adoption opens a never-opened same-boot lane in one round trip`() =
        runBlocking {
            val client = FakeMutationClient()
            val io = ConfigRootIo { client }
            assertEquals(ConfigInitialization(ConfigCoordinatorMode.Open, CanonicalConfig()), io.initialize())
            assertEquals(1, client.opens)
            // A quiescent lane from a previous boot is adopted the same way.
            client.snapshot = client.snapshot.copy(receipt = client.snapshot.receipt.copy(boot = OLD_BOOT, session = null))
            assertEquals(ConfigCoordinatorMode.Open, ConfigRootIo { client }.initialize().mode)
            assertEquals(2, client.opens)
        }

    @Test
    fun `same boot running predecessor stays paused even with readable data`() =
        runBlocking {
            val client = FakeMutationClient()
            client.snapshot =
                client.snapshot.copy(
                    receipt = client.snapshot.receipt.copy(session = OLD_BOOT, sequence = 1, status = RootReceiptStatus.Running),
                )
            val io = ConfigRootIo { client }
            assertEquals(ConfigCoordinatorMode.Paused, io.initialize().mode)
            assertEquals(0, client.opens)
        }

    @Test
    fun `root effect and recovery identities are retained and warning reaches existing parser`() =
        runBlocking {
            val client = adoptedClient()
            var preparations = 0
            val io =
                ConfigRootIo {
                    preparations += 1
                    client
                }
            assertEquals(ConfigCoordinatorMode.Open, io.initialize().mode)
            val candidate = CanonicalConfig(debugSwitch = true)
            assertEquals(
                PhaseOutcome.Confirmed,
                io.execute(EffectTicket(1, 2), ConfigPhase.Persist, CanonicalConfig(), candidate, "persist").outcome,
            )
            assertEquals(PhaseOutcome.Unknown, io.recover(2, ConfigPhase.Persist).outcome)
            assertEquals(PhaseOutcome.Unknown, io.recover(1, ConfigPhase.Native).outcome)
            assertEquals(0, client.recoveries)
            assertEquals(PhaseOutcome.Confirmed, io.recover(1, ConfigPhase.Persist).outcome)
            client.warning = NativeTargetCapacityWarning(10, 8, 2)
            val native = io.execute(EffectTicket(1, 3), ConfigPhase.Native, candidate, candidate, "activate")
            assertEquals(client.warning, parseNativeTargetCapacityWarning(native.output))
            assertEquals(native, io.recover(1, ConfigPhase.Native))
            assertEquals(2, client.executions)
            assertEquals(2L, client.snapshot.receipt.sequence)
            assertEquals(1, preparations)
        }

    @Test
    fun `missing failed bootstrap is known but wrong boot cannot resolve it`() =
        runBlocking {
            val client = adoptedClient()
            client.snapshot = client.snapshot.copy(canonical = RootCanonicalRead.Missing)
            val io = ConfigRootIo { client }
            assertEquals(ConfigCoordinatorMode.Missing, io.initialize().mode)
            client.replaceConfig = false
            val failed = io.execute(EffectTicket(1, 2), ConfigPhase.Persist, null, CanonicalConfig(), "fail")
            assertEquals(PhaseOutcome.FailedKnown, failed.outcome)
            assertEquals(RootCanonicalRead.Missing, failed.canonical)
            client.snapshot = client.snapshot.copy(boot = OLD_BOOT)
            assertEquals(RootCanonicalRead.Unavailable, io.read())
            assertEquals(PhaseOutcome.Unknown, io.recover(1, ConfigPhase.Persist).outcome)
        }

    @Test
    fun `new client session fences observations from earlier session`() =
        runBlocking {
            val client = adoptedClient()
            val io = ConfigRootIo { client }
            io.initialize()
            assertTrue(io.read() is RootCanonicalRead.Available)
            client.snapshot = client.snapshot.copy(receipt = client.snapshot.receipt.copy(session = OLD_BOOT))
            assertEquals(RootCanonicalRead.Unavailable, io.read())
        }
}

private const val BOOT = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
private const val OLD_BOOT = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

private fun adoptedClient() = FakeMutationClient().apply { snapshot = snapshot.copy(receipt = snapshot.receipt.copy(boot = OLD_BOOT)) }

private class FakeMutationClient : RootMutationClient {
    var snapshot =
        RootMutationSnapshot(
            BOOT,
            RootMutationReceipt(0, BOOT, null, 0, RootReceiptStatus.Idle, null, false),
            RootCanonicalRead.Available(CanonicalConfig()),
        )
    var opens = 0
    var executions = 0
    var recoveries = 0
    var replaceConfig = true
    var warning: NativeTargetCapacityWarning? = null

    override fun inspect() = RootMutationReply.Observed(true, snapshot)

    override fun open(
        expected: RootMutationSnapshot,
        sessionId: String,
    ): RootMutationReply {
        opens += 1
        snapshot =
            snapshot.copy(
                receipt = RootMutationReceipt(expected.receipt.revision + 1, BOOT, sessionId, 0, RootReceiptStatus.Idle, null, false),
            )
        return inspect()
    }

    /** Mirrors vhmutate's adoption policy: a rejected adoption still returns the receipt. */
    override fun adopt(sessionId: String): RootMutationReply {
        val receipt = snapshot.receipt
        if (receipt.session == sessionId && receipt.boot == snapshot.boot) return inspect()
        if (!snapshot.quiescent) return RootMutationReply.Observed(false, snapshot)
        return open(snapshot, sessionId)
    }

    override fun execute(
        session: RootMutationSession,
        sequence: Long,
        command: String,
    ): RootMutationReply {
        executions += 1
        check(snapshot.receipt.session == session.id && sequence == snapshot.receipt.sequence + 1)
        snapshot =
            snapshot.copy(
                receipt =
                    snapshot.receipt.copy(
                        revision = snapshot.receipt.revision + 2,
                        sequence = sequence,
                        status = RootReceiptStatus.Finished,
                        exitCode = 0,
                        nativeCapacity = warning,
                    ),
                canonical = if (replaceConfig) RootCanonicalRead.Available(CanonicalConfig(debugSwitch = true)) else snapshot.canonical,
            )
        return inspect()
    }

    override fun recover(
        session: RootMutationSession,
        sequence: Long,
    ): RootMutationReply {
        recoveries += 1
        check(snapshot.receipt.session == session.id && snapshot.receipt.sequence == sequence)
        return inspect()
    }
}
