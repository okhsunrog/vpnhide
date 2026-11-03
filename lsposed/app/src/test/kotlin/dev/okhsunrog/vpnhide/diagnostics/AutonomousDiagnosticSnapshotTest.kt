package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.CoreSessionUpdate
import dev.okhsunrog.vpnhide.CoreStateView
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.ObservedValue
import dev.okhsunrog.vpnhide.ReadReason
import dev.okhsunrog.vpnhide.RootCanonicalRead
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.belongsTo
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreAssessmentPhase
import uniffi.vpnhide_core_ffi.CoreAttachmentEvidence
import uniffi.vpnhide_core_ffi.CoreDiagnosticAssessment
import uniffi.vpnhide_core_ffi.CoreDiagnosticOutcome
import uniffi.vpnhide_core_ffi.CoreDiagnosticRun
import uniffi.vpnhide_core_ffi.CoreDiagnosticThreadIdentity
import uniffi.vpnhide_core_ffi.CoreNativeRawReceipt

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
class AutonomousDiagnosticSnapshotTest {
    private fun assessment() =
        CoreDiagnosticAssessment(
            CoreAssessmentPhase.RUNNING,
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            emptyList(),
            null,
            null,
        )

    private fun run(epoch: Long) =
        CoreDiagnosticRun(
            nativeRaw = null,
            session = 1uL,
            coreEpoch = epoch.toULong(),
            bootId = "boot",
            policySha256 = "a".repeat(64),
            runtimeSha256 = "b".repeat(64),
            platformSha256 = null,
            loadedGenerationsSha256 = null,
            loadedGenerationsComplete = false,
            contract = 3u,
            required = emptyList(),
            outcome = CoreDiagnosticOutcome.Running,
            framework = null,
            stale = false,
            binding = null,
            addresses = null,
            procInterfaces = null,
        )

    private fun update(
        epoch: Long = 7,
        sequence: Long = 3,
    ) = CoreSessionUpdate(
        CoreStateView(
            epoch,
            0,
            RootCanonicalRead.Missing,
            emptyList(),
            autonomousDiagnostics = run(epoch),
            autonomousAssessment = assessment(),
        ),
        current = true,
        sequence = sequence,
        connectionGeneration = 1,
    )

    private fun config(core: CoreSessionUpdate) =
        ConfigCoordinatorView(coreCurrent = true, coreEpoch = core.state?.epoch, desiredRevision = 0, coreSequence = core.sequence)

    private fun observed(root: RootSnapshot) =
        ObservationState(lastGood = ObservedValue(root, ObservationRequest(root.observationId, 0, 0, ReadReason.Background), 0))

    @Test
    fun `Dashboard root projection requires both observation and generation`() {
        val facts = dev.okhsunrog.vpnhide.RootProjection(2, 3, "facts")
        val root = RootSnapshot(emptyMap(), observationId = 2, generation = 3)
        assertTrue(facts.belongsTo(root))
        assertFalse(facts.belongsTo(root.copy(observationId = 1)))
        assertFalse(facts.belongsTo(root.copy(generation = 4)))
        assertFalse(facts.belongsTo(null))
    }

    @Test
    fun `new Core epoch cannot borrow old root facts or old worker run`() {
        val old = update()
        val newer = update(epoch = 8, sequence = 4)
        val root = RootSnapshot(emptyMap(), observationId = 1, core = old)
        val sources = AutonomousDiagnosticSources(newer, observed(root), config(newer))
        val snapshot = autonomousDiagnosticSnapshot(sources, 1, 0)
        assertTrue(snapshot.current)
        assertFalse(snapshot.rootJoined)
        val wrongRun = newer.copy(state = newer.state?.copy(autonomousDiagnostics = run(7)))
        val mismatched = autonomousDiagnosticSnapshot(sources.copy(core = wrongRun), 2, 0)
        assertEquals(AutonomousAvailability.EpochMismatch, mismatched.availability)
        assertFalse(mismatched.current)
    }

    @Test
    fun `config actor acceptance and freshness are independent of scoped coverage`() {
        val core = update()
        val sources = AutonomousDiagnosticSources(core, ObservationState(), config(core).copy(desiredRevision = 1))
        assertEquals(AutonomousAvailability.ConfigUnjoined, autonomousDiagnosticSnapshot(sources, 0, 0).availability)
        val accepted = autonomousDiagnosticSnapshot(sources.copy(config = config(core)), 1, 0)
        assertTrue(accepted.current)
        assertFalse(requireNotNull(accepted.run).loadedGenerationsComplete)
        val unpaired = core.copy(state = core.state?.copy(autonomousAssessment = null))
        val unavailable = autonomousDiagnosticSnapshot(sources.copy(core = unpaired, config = config(core)), 2, 0)
        assertEquals(AutonomousAvailability.IncompletePublication, unavailable.availability)
        assertFalse(unavailable.current)
        val stale = core.copy(state = core.state?.copy(autonomousDiagnostics = run(7).copy(stale = true)))
        assertFalse(autonomousDiagnosticSnapshot(sources.copy(core = stale, config = config(core)), 2, 0).current)
    }

    @Test
    fun `same revision does not join different accepted canonical contents`() {
        val core = update()
        val different = config(core).copy(confirmed = dev.okhsunrog.vpnhide.CanonicalConfig(debug = true))
        val snapshot = autonomousDiagnosticSnapshot(AutonomousDiagnosticSources(core, ObservationState(), different), 0, 0)
        assertEquals(AutonomousAvailability.ConfigUnjoined, snapshot.availability)
        assertFalse(snapshot.current)
    }

    @Test
    fun `same config revision cannot join changed runtime with older root facts`() {
        val initial = update()
        val root = RootSnapshot(emptyMap(), observationId = 1, core = initial)
        val changed =
            initial.copy(
                state =
                    initial.state?.copy(
                        revision = 2,
                        backend =
                            dev.okhsunrog.vpnhide.CoreBackendView(
                                current = true,
                                selected = dev.okhsunrog.vpnhide.NativeBackendId.Kmod,
                                applied = dev.okhsunrog.vpnhide.NativeBackendId.Kmod,
                            ),
                    ),
                sequence = 4,
            )
        val snapshot = autonomousDiagnosticSnapshot(AutonomousDiagnosticSources(changed, observed(root), config(changed)), 1, 0)
        assertFalse(snapshot.rootJoined)
        // A diagnostic publication alone does not invalidate unchanged root facts.
        val newRun =
            initial.copy(
                state = initial.state?.copy(revision = 3, autonomousDiagnostics = run(7).copy(session = 2uL)),
                sequence = 5,
            )
        assertTrue(autonomousDiagnosticSnapshot(AutonomousDiagnosticSources(newRun, observed(root), config(newRun)), 2, 0).rootJoined)
    }

    @Test
    fun `receipt updates do not invalidate unrelated heavy root metadata`() {
        val initial = update()
        val root = RootSnapshot(emptyMap(), observationId = 1, core = initial)
        val changed =
            initial.copy(
                sequence = 4,
                state =
                    initial.state?.copy(
                        revision = 2,
                        operationsInFlight = listOf(17),
                        activeCaptures = 1,
                    ),
            )
        assertTrue(autonomousDiagnosticSnapshot(AutonomousDiagnosticSources(changed, observed(root), config(changed)), 1, 0).rootJoined)
    }

    @Test
    fun `foreign VPN state and split tunnel membership cannot gate retained diagnostics`() {
        val initial = update()
        val root = RootSnapshot(emptyMap(), observationId = 1, core = initial)
        val accepted = config(initial)
        val before = autonomousDiagnosticSnapshot(AutonomousDiagnosticSources(initial, observed(root), accepted), 0, 0)
        for ((vpn, included) in listOf(false to false, true to false, true to true, false to true)) {
            // Raw forensic data may change; neither value is a diagnostic prerequisite.
            val sections = mapOf("vpn_present" to vpn.toString(), "apk_in_vpn" to included.toString())
            val changed = initial.copy(sequence = initial.sequence + 1)
            val after =
                autonomousDiagnosticSnapshot(
                    AutonomousDiagnosticSources(changed, observed(root.copy(sections = sections)), accepted),
                    1,
                    0,
                )
            assertTrue(after.current)
            assertTrue(after.rootJoined)
            assertEquals(before.run, after.run)
            assertEquals(before.assessment, after.assessment)
            assertEquals(before.availability, after.availability)
        }
    }

    @Test
    fun `published evidence does not alias mutable transport records`() {
        val initial = update()
        val snapshot = autonomousDiagnosticSnapshot(AutonomousDiagnosticSources(initial, ObservationState(), config(initial)), 1, 0)
        initial.state?.autonomousDiagnostics?.session = 99uL
        initial.state?.autonomousAssessment?.phase = CoreAssessmentPhase.INFRASTRUCTURE
        assertEquals(1L, snapshot.run?.session)
        assertEquals("running", snapshot.assessment?.phase)
    }

    @Test
    fun `disconnect releases root waiter with retained evidence qualified unavailable`() =
        runTest {
            val initial = update()
            val core = MutableStateFlow(initial)
            val root = MutableStateFlow<ObservationState<RootSnapshot>>(ObservationState())
            val config = MutableStateFlow(config(initial))
            val owner = AutonomousDiagnosticSnapshotOwner(backgroundScope, core, root, config) { 0 }
            val waiter = async(start = CoroutineStart.UNDISPATCHED) { owner.awaitSnapshot(RootSnapshot(emptyMap(), observationId = 5)) }
            runCurrent()
            assertFalse(waiter.isCompleted)
            core.value = initial.copy(current = false, sequence = 4, failureDetail = "disconnected")
            runCurrent()
            val unavailable = waiter.await()
            assertEquals(AutonomousAvailability.Offline, unavailable.availability)
            assertFalse(unavailable.current)
            assertEquals(initial.state?.autonomousDiagnostics?.toDiagnosticInfo(), unavailable.run)
        }

    @Test
    fun `cancelled reader does not cancel owner and accepted newer root satisfies checkpoint`() =
        runTest {
            val initial = update()
            val core = MutableStateFlow(initial)
            val root = MutableStateFlow<ObservationState<RootSnapshot>>(ObservationState())
            val config = MutableStateFlow(config(initial))
            val owner = AutonomousDiagnosticSnapshotOwner(backgroundScope, core, root, config) { 0 }
            val abandoned = async { owner.awaitSnapshot(RootSnapshot(emptyMap(), observationId = 4)) }
            runCurrent()
            abandoned.cancel()
            val published = RootSnapshot(emptyMap(), observationId = 6, core = initial)
            root.value = observed(published)
            runCurrent()
            val accepted = owner.awaitSnapshot(RootSnapshot(emptyMap(), observationId = 5))
            assertSame(published, accepted.rootSnapshot)
            assertTrue(accepted.rootJoined)
        }

    @Test
    fun `export adapter uses typed Rust advice and withholds unjoined diagnostic evidence`() {
        val initial = update()
        val core = initial.copy(state = initial.state?.copy(attachmentEvidence = CoreAttachmentEvidence(false, false, true, true)))
        val sources = AutonomousDiagnosticSources(core, ObservationState(), config(core))
        val advice = exportRecommendation(autonomousDiagnosticSnapshot(sources, 1, 0))
        assertTrue(advice.kernel && advice.java)
        assertEquals(ExportReason.KERNEL_HOOKS, advice.kernelReason)
        assertEquals(ExportReason.JAVA_HOOKS, advice.javaReason)
        val unjoined = autonomousDiagnosticSnapshot(sources.copy(config = config(core).copy(desiredRevision = 2)), 2, 0)
        val withheld = exportRecommendation(unjoined)
        assertFalse(withheld.kernel || withheld.java)
        val disconnected = autonomousDiagnosticSnapshot(sources.copy(core = core.copy(current = false)), 3, 0)
        assertEquals(ExportReason.CORE_UNAVAILABLE, exportRecommendation(disconnected).kernelReason)
        assertEquals(false to true, ExportOverrides(kernel = false).resolve(advice))
    }

    @Test
    fun `export adapter keeps boot compatibility advice without launching a diagnostic run`() {
        val initial = update()
        val core =
            initial.copy(
                state =
                    initial.state?.copy(
                        autonomousDiagnostics = null,
                        attachmentEvidence = CoreAttachmentEvidence(false, true, false, false),
                    ),
            )
        val sources = AutonomousDiagnosticSources(core, ObservationState(), config(core))
        val snapshot = autonomousDiagnosticSnapshot(sources, 1, 0)
        assertFalse(snapshot.current)
        val advice = exportRecommendation(snapshot)
        assertTrue(advice.kernel)
        assertFalse(advice.java)
        assertEquals(ExportReason.KERNEL_COMPATIBILITY, advice.kernelReason)
    }

    @Test
    fun `forensic export preserves the Core raw receipt and exact thread identity`() {
        val source =
            run(7).copy(
                nativeRaw =
                    CoreNativeRawReceipt.CapabilityAbsent(
                        CoreDiagnosticThreadIdentity(10000u, 42u, 100uL, 43u, 101uL),
                    ),
            )
        val raw = requireNotNull(source.toDiagnosticInfo().nativeRaw).jsonObject
        assertEquals("capability_absent", raw.getValue("state").jsonPrimitive.content)
        val identity = raw.getValue("identity").jsonObject
        assertEquals("43", identity.getValue("tid").jsonPrimitive.content)
        assertEquals("101", identity.getValue("thread_start_ticks").jsonPrimitive.content)
        assertEquals("10000", identity.getValue("uid").jsonPrimitive.content)
    }
}
