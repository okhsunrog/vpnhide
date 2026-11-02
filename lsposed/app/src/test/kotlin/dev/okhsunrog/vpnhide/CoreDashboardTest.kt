package dev.okhsunrog.vpnhide

import org.junit.Assert.*
import org.junit.Test

class CoreDashboardTest {
    private fun state(ports: Boolean = false) =
        CoreStateView(
            epoch = 1,
            configRevision = 2,
            canonical = RootCanonicalRead.Available(CanonicalConfig(apps = mapOf("example" to CanonicalApp(ports = ports)))),
            operationsInFlight = emptyList(),
        )

    private fun observed(state: CoreStateView = state()) = CoreSessionUpdate(state, current = true)

    @Test fun `core health is independent of ports and installed metadata`() {
        val result = coreDashboardData(observed())
        assertEquals(CoreHealth.Ready, result.health)
        assertNull(result.installedVersion)
        assertEquals(0, result.portsTargets)
        assertEquals(CorePortsHealth.Unknown, result.ports)
        assertEquals(CoreHealth.Ready, coreDashboardData(observed(state(true)), "old installed version").health)
    }

    @Test fun `unavailable stale mismatch and permission failures are never healthy`() {
        assertEquals(CoreHealth.Connecting, coreDashboardData(CoreSessionUpdate()).health)
        assertEquals(CoreHealth.Unavailable, coreDashboardData(CoreSessionUpdate(sequence = 1)).health)
        val stale = observed(state(true)).copy(current = false)
        assertEquals(CoreHealth.Stale, coreDashboardData(stale).health)
        assertEquals(CorePortsHealth.Unknown, coreDashboardData(stale).ports)
        assertNull(coreDashboardData(stale).portsTargets)
        assertEquals(CoreHealth.ProtocolMismatch, coreDashboardData(stale.copy(protocolSkew = true)).health)
        assertEquals(CoreHealth.PermissionDenied, coreDashboardData(stale.copy(failureDetail = "Permission denied (os error 13)")).health)
    }

    @Test fun `transport connection does not imply operational readiness`() {
        assertEquals(CoreHealth.StorageFault, coreDashboardData(observed(state().copy(storageFault = "journal unavailable"))).health)
        assertEquals(CoreHealth.StartupFault, coreDashboardData(observed(state().copy(startupError = "invalid canonical"))).health)
        assertEquals(CoreHealth.Paused, coreDashboardData(observed(state().copy(mutationHeld = true))).health)
        assertEquals(CoreHealth.Paused, coreDashboardData(observed(state().copy(heldResources = setOf(ConfigPhase.Ports)))).health)
        assertEquals(
            CoreHealth.ConfigurationRequired,
            coreDashboardData(observed(state().copy(canonical = RootCanonicalRead.Missing))).health,
        )
    }

    @Test fun `ports delivery must match desired revision including removal`() {
        val delivery = CoreApplyIdentity(ConfigPhase.Ports, 2, 9, PhaseOutcome.Confirmed)
        val enabled = state(true).copy(applyEvidence = listOf(delivery))
        assertEquals(CorePortsHealth.Applied, coreDashboardData(observed(enabled)).ports)
        assertEquals(CorePortsHealth.Disabled, coreDashboardData(observed(enabled.copy(canonical = state().canonical))).ports)
        assertEquals(CorePortsHealth.Unknown, coreDashboardData(observed(enabled.copy(configRevision = 3))).ports)
        assertEquals(CorePortsHealth.Held, coreDashboardData(observed(enabled.copy(heldResources = setOf(ConfigPhase.Ports)))).ports)
        assertEquals(
            CorePortsHealth.Failed,
            coreDashboardData(observed(enabled.copy(applyEvidence = listOf(delivery.copy(outcome = PhaseOutcome.FailedKnown))))).ports,
        )
    }

    @Test fun `failure details obey publication fencing and clear on reconnect`() {
        val fresh = acceptCorePublication(CorePublicationData(), 1, 2, state())
        val oldFailure = failCorePublication(fresh, 1, 1, false, "old permission failure")
        assertEquals(fresh, oldFailure)
        val failed = failCorePublication(fresh, 1, 3, false, "Permission denied")
        assertEquals(CoreHealth.PermissionDenied, coreDashboardData(failed.update).health)
        val recovered = acceptCorePublication(failed, 2, 4, state())
        assertEquals(CoreHealth.Ready, coreDashboardData(recovered.update).health)
        assertNull(recovered.update.failureDetail)
    }
}
