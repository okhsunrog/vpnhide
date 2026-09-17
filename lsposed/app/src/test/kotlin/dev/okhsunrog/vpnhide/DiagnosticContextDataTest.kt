package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.CORE_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResult
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.ConfigReadiness
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttempt
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.EXTRA_JAVA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.NATIVE_EXTRA_CHECKS
import dev.okhsunrog.vpnhide.diagnostics.RoutingRead
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
import dev.okhsunrog.vpnhide.diagnostics.SelfRouting
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticContextObservation
import dev.okhsunrog.vpnhide.diagnostics.diagnosticProbePlan
import dev.okhsunrog.vpnhide.diagnostics.diagnosticRequest
import dev.okhsunrog.vpnhide.diagnostics.mergeDiagnosticEvidence
import dev.okhsunrog.vpnhide.diagnostics.plannedOutcomes
import dev.okhsunrog.vpnhide.diagnostics.routingReadPlan
import dev.okhsunrog.vpnhide.diagnostics.selfConfigurationIdentity
import dev.okhsunrog.vpnhide.diagnostics.selfRoutingObservation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DiagnosticContextDataTest {
    private val selfPackage = "dev.okhsunrog.vpnhide"

    @Test
    fun `restart requirement blocks before routing is consulted`() {
        val observation = observe(selfNeedsRestart = true, routing = current(DiagnosticGate.ROUTED))
        assertEquals(DiagnosticEligibility.RestartApp, observation.eligibility)
    }

    @Test
    fun `failed routing read is unknown without a comparable context even with historical routed value`() {
        val failed =
            ObservationState(
                lastGood = ObservedValue(DiagnosticGate.ROUTED, ObservationRequest(1, 0, 0, ReadReason.Explicit), 5),
                generation = 1,
                error = TransitionFailure.ReadFailed,
                attempted = true,
                attemptedGeneration = 1,
            )
        val observation = observe(routing = failed)
        assertEquals(DiagnosticEligibility.Unknown, observation.eligibility)
        assertNull(observation.context)
        assertNull(selfRoutingObservation(current(DiagnosticGate.NEEDS_RESTART)).lastGood)
    }

    /**
     * The projection rebuilds the state field by field, so every field a consumer
     * reads has to be carried: the stale mark is what tells the presentation why a
     * re-read is owed and since when, and dropping it silently degrades every
     * re-read to Background/now.
     */
    @Test
    fun `the self routing projection carries the stale mark of the gate observation`() {
        val mark = StaleMark(ReadReason.Transition, since = 11)
        val invalidated = current(DiagnosticGate.ROUTED).copy(generation = 1, stale = mark)
        val projected = selfRoutingObservation(invalidated)
        assertEquals(mark, projected.stale)
        assertEquals(SelfRouting.Routed, projected.lastGood?.value)
        assertNull(selfRoutingObservation(current(DiagnosticGate.ROUTED)).stale)
    }

    @Test
    fun `routing read reuses a current observation joins an in-flight read and refreshes only stale state`() {
        val fresh = current(DiagnosticGate.ROUTED)
        assertEquals(RoutingRead.Reuse, routingReadPlan(fresh))
        assertEquals(RoutingRead.Refresh, routingReadPlan(ObservationState<DiagnosticGate>()))
        assertEquals(RoutingRead.Refresh, routingReadPlan(fresh.copy(generation = 1)))
        assertEquals(RoutingRead.Refresh, routingReadPlan(fresh.copy(error = TransitionFailure.ReadFailed)))
        assertEquals(RoutingRead.Refresh, routingReadPlan(fresh.copy(quarantined = true, quarantineRequestId = 1)))
        val loading = fresh.copy(active = ObservationRequest(2, 0, 1, ReadReason.Transition))
        assertEquals(RoutingRead.Join, routingReadPlan(loading))
        assertEquals(RoutingRead.Join, routingReadPlan(loading.copy(generation = 1)))
    }

    @Test
    fun `config readiness and change epoch from the impact state reach eligibility and the context`() {
        val applying =
            buildDiagnosticContextObservation(
                selfNeedsRestart = false,
                routing = current(DiagnosticGate.ROUTED),
                snapshot = RootSnapshot(snapshotSections(), observationId = 7, generation = 3),
                config = CanonicalConfig(),
                selfPackage = selfPackage,
                processIdentity = "pid:1;uid:10",
                now = 42,
                readiness = ConfigReadiness.Applying,
                changeEpoch = 3,
            )
        assertEquals(DiagnosticEligibility.Applying, applying.eligibility)
        assertEquals(3L, applying.context?.changeEpoch)
        val unknown = observe(routing = current(DiagnosticGate.ROUTED), readiness = ConfigReadiness.Unknown)
        assertEquals(DiagnosticEligibility.ApplicationUnknown, unknown.eligibility)
        val failed = observe(routing = current(DiagnosticGate.ROUTED), readiness = ConfigReadiness.Failed)
        assertEquals(DiagnosticEligibility.ApplicationFailed, failed.eligibility)
    }

    @Test
    fun `routed observation is eligible and carries snapshot and config identities`() {
        val observation = observe(routing = current(DiagnosticGate.ROUTED))
        assertEquals(DiagnosticEligibility.Eligible, observation.eligibility)
        val context = requireNotNull(observation.context)
        assertEquals("pid:1;uid:10;boot:boot-1", context.subject)
        assertEquals("vpn=tun0;self=ROUTED", context.routing)
        assertEquals("backend=null;active=false;hooks=[];lsposed=false", context.coverage)
        // The typed layers behind the identity travel with the measurement so its
        // report is built against them later, whatever backend is active by then.
        val layers = requireNotNull(context.coverageLayers)
        assertEquals(context.coverage, layers.identity)
        assertEquals(null, layers.backend.id)
        assertEquals(false, layers.lsposedActive)
        assertEquals(7L, context.observationId)
        assertEquals(0L, context.changeEpoch)
        assertEquals(42L, context.observedAt)
        val blocked = observe(routing = current(DiagnosticGate.VPN_OFF))
        assertEquals(DiagnosticEligibility.VpnOff, blocked.eligibility)
        assertEquals("vpn=tun0;self=VPN_OFF", blocked.context?.routing)
    }

    @Test
    fun `self configuration identity ignores unrelated apps and debug flags`() {
        val base =
            CanonicalConfig(
                apps =
                    mapOf(
                        selfPackage to CanonicalApp(java = true),
                        "other.app" to CanonicalApp(native = NativeRole.All),
                    ),
            )
        val identity = selfConfigurationIdentity(base, selfPackage)
        assertEquals(identity, selfConfigurationIdentity(base.copy(debug = true, debugSwitch = true), selfPackage))
        assertEquals(
            identity,
            selfConfigurationIdentity(base.copy(apps = base.apps + ("other.app" to CanonicalApp(ports = true))), selfPackage),
        )
        assertNotEquals(
            identity,
            selfConfigurationIdentity(base.copy(apps = base.apps + (selfPackage to CanonicalApp(java = false))), selfPackage),
        )
        assertNotEquals(
            identity,
            selfConfigurationIdentity(base.copy(settings = CanonicalSettings(optionalFeatures = setOf("fs"))), selfPackage),
        )
        assertNotEquals(identity, selfConfigurationIdentity(null, selfPackage))
    }

    @Test
    fun `probe plan lists every registered check once and outcomes only enter by planned id`() {
        val plan = diagnosticProbePlan()
        val expected =
            NATIVE_CHECKS.map { it.id } + NATIVE_EXTRA_CHECKS.map { it.id } + CORE_JAVA_CHECKS.map { it.id } +
                EXTRA_JAVA_CHECKS.map { it.id }
        assertEquals(expected, plan.map { it.id })
        assertEquals(expected.size, expected.toSet().size)
        assertTrue(plan.filter { !it.owned }.map { it.id } == NATIVE_EXTRA_CHECKS.map { it.id })
        assertEquals(plan, diagnosticRequest(automatic = true).plan)

        val checks =
            listOf(
                CheckResult("labelled", "", CheckOutcome.Leak, id = "ioctl_flags"),
                CheckResult("unplanned", "", CheckOutcome.Leak, id = "not_a_probe"),
                CheckResult("unlabeled", "", CheckOutcome.Leak),
            )
        assertEquals(mapOf("ioctl_flags" to CheckOutcome.Leak), plannedOutcomes(plan, checks))
    }

    @Test
    fun `evidence merge keeps core results and adds only the slow list`() {
        val core = CheckResults(native = listOf(CheckResult("n", "", CheckOutcome.HiddenByBackend, id = "ioctl_flags")))
        val slow = CheckResults(native = emptyList(), extraJava = listOf(CheckResult("j", "", CheckOutcome.Leak, id = "network_callback")))
        assertEquals(core, mergeDiagnosticEvidence(null, DiagnosticStage.Core, core))
        assertEquals(core.copy(extraJava = slow.extraJava), mergeDiagnosticEvidence(core, DiagnosticStage.Slow, slow))
        assertEquals(slow, mergeDiagnosticEvidence(null, DiagnosticStage.Slow, slow))
    }

    private fun current(gate: DiagnosticGate): ObservationState<DiagnosticGate> =
        ObservationState(
            lastGood = ObservedValue(gate, ObservationRequest(1, 0, 0, ReadReason.Explicit), 5),
            attempted = true,
            attemptedGeneration = 0,
        )

    private fun observe(
        selfNeedsRestart: Boolean = false,
        routing: ObservationState<DiagnosticGate>,
        readiness: ConfigReadiness = ConfigReadiness.Settled,
    ) = buildDiagnosticContextObservation(
        selfNeedsRestart = selfNeedsRestart,
        routing = routing,
        snapshot = RootSnapshot(snapshotSections(), observationId = 7, generation = 3),
        config = CanonicalConfig(),
        selfPackage = selfPackage,
        processIdentity = "pid:1;uid:10",
        now = 42,
        readiness = readiness,
    )

    private fun snapshotSections(): Map<String, String> =
        mapOf(
            "current_boot_id" to "boot-1\n",
            "vpn_networks" to
                "Current Networks:\n  NetworkAgentInfo{ni{VPN CONNECTED} lp{InterfaceName: tun0} " +
                "nc{[ Transports: VPN Capabilities: INTERNET ]}}\n",
            "vpn_routes4" to "probe_ok",
            "vpn_routes6" to "probe_ok",
        )
}
