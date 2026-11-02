package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test

class DetectModulesTest {
    @Test
    fun `ports runtime unverified when snapshot shell lacked root`() {
        val sections =
            mapOf(
                "core_prop" to "version=1.2.3",
                "ports_chain" to "0",
                "snapshot_shell_uid" to "uid=2000\nid=uid=2000(shell)\ncontext=u:r:shell:s0",
            )
        val state = detectPortsModule(sections) as ModuleState.Installed
        assertEquals(false, state.active)
        assertEquals(false, state.runtimeCheckable)
    }

    @Test
    fun `ports active when iptables probe reports connected chains`() {
        val sections =
            mapOf(
                "core_prop" to "version=0.6.3",
                "ports_chain" to "1",
            )
        val state = detectPortsModule(sections) as ModuleState.Installed
        assertEquals(true, state.active)
    }

    @Test
    fun `ports inactive when iptables probe reports missing chains or jumps`() {
        val sections =
            mapOf(
                "core_prop" to "version=0.6.3",
                "ports_chain" to "0",
                "snapshot_shell_uid" to "uid=0\nid=uid=0(root)\ncontext=u:r:ksu:s0",
            )
        val state = detectPortsModule(sections) as ModuleState.Installed
        assertEquals(false, state.active)
        assertEquals(true, state.runtimeCheckable)
    }

    @Test
    fun `ports unverified, not inactive, when the iptables probe itself failed`() {
        // Exit 4: another process held the xtables lock for longer than the probe waited.
        val locked = detectPortsModule(mapOf("core_prop" to "version=0.6.3", "ports_chain" to "error=4")) as ModuleState.Installed
        assertEquals(false, locked.active)
        assertEquals(false, locked.runtimeCheckable)
        // The phase never emitted anything.
        val missing = detectPortsModule(mapOf("core_prop" to "version=0.6.3")) as ModuleState.Installed
        assertEquals(false, missing.runtimeCheckable)
        // An unverified reading is no evidence that the rules are missing: no warning.
        assertEquals(null, detectPortsApplyProblem(locked, 1, portsApply(), portsDisabled = false))
    }

    private fun portsApply(
        ipv4: FamilyOutcome = FamilyOutcome.Confirmed,
        ipv6: FamilyOutcome = FamilyOutcome.Confirmed,
        detail: String = "",
    ) = PortsBootStatus(PortsBootSource.Boot, ipv4, ipv6, targetCount = 1, netdReady = true, detail = detail, atMs = 1)

    @Test
    fun `ports apply problem carries core's boot failure detail`() {
        val ports = ModuleState.Installed(version = "2.0.0", active = false)
        val problem =
            detectPortsApplyProblem(
                ports,
                1,
                portsApply(ipv4 = FamilyOutcome.FailedKnown, detail = "iptables-restore failed"),
                portsDisabled = false,
            )

        assertEquals("iptables-restore failed", problem?.failureDetail)
        val unknown = detectPortsApplyProblem(ports, 1, portsApply(ipv6 = FamilyOutcome.Unknown), portsDisabled = false)
        assertEquals("IPv4 Confirmed, IPv6 Unknown", unknown?.failureDetail)
    }

    @Test
    fun `ports apply problem is generic when chains are missing after a clean boot apply`() {
        val ports = ModuleState.Installed(version = "2.0.0", active = false)

        assertEquals(null, detectPortsApplyProblem(ports.copy(active = true), 1, null, portsDisabled = false))
        assertEquals(null, detectPortsApplyProblem(ports, 0, null, portsDisabled = false))
        assertEquals(
            PortsApplyProblem(failureDetail = null),
            detectPortsApplyProblem(ports, 1, portsApply(), portsDisabled = false),
        )
        assertEquals(PortsApplyProblem(failureDetail = null), detectPortsApplyProblem(ports, 1, null, portsDisabled = false))
    }

    @Test
    fun `ports apply problem is suppressed for a deliberately disabled core module`() {
        val ports = ModuleState.Installed(version = "2.0.0", active = false)

        assertEquals(
            null,
            detectPortsApplyProblem(ports, 1, portsApply(ipv4 = FamilyOutcome.FailedKnown), portsDisabled = true),
        )
    }

    @Test
    fun `ports apply problem does not duplicate a module integrity failure`() {
        val ports =
            ModuleState.Installed(
                version = "2.0.0",
                active = false,
                brokenReason = ModuleBrokenReason.ArtifactMissing,
            )

        assertEquals(null, detectPortsApplyProblem(ports, 1, null, portsDisabled = false))
    }

    @Test
    fun `ports not available when the core prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectPortsModule(mapOf("ports_chain" to "1")))
    }
}
