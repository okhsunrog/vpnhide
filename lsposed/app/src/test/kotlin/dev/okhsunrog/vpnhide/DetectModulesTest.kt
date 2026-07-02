package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test

class DetectModulesTest {
    private val self = "dev.okhsunrog.vpnhide"

    @Test
    fun `kmod not installed when prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectKmodModule(emptyMap(), self))
    }

    @Test
    fun `kmod active when proc node present, target count excludes self`() {
        val sections =
            mapOf(
                "kmod_prop" to "version=0.6.3\ngkiVariant=android13-5.10",
                "proc_exists" to "1",
                "kmod_targets" to "$self\ncom.bank.app\n",
            )
        val state = detectKmodModule(sections, self) as ModuleState.Installed
        assertEquals("0.6.3", state.version)
        assertEquals(true, state.active)
        assertEquals("android13-5.10", state.gkiVariant)
        assertEquals(1, state.targetCount)
    }

    @Test
    fun `kmod inactive when proc node absent`() {
        val sections = mapOf("kmod_prop" to "version=0.6.3", "proc_exists" to "0")
        assertEquals(false, (detectKmodModule(sections, self) as ModuleState.Installed).active)
    }

    @Test
    fun `zygisk active only when heartbeat matches current boot`() {
        val sections = mapOf("zygisk_prop" to "version=0.6.3", "zygisk_targets" to "$self\ncom.chat.app\n")
        val fresh = detectZygiskModule(sections, "boot_id=boot-1", self, "boot-1") as ModuleState.Installed
        assertEquals(true, fresh.active)
        assertEquals(1, fresh.targetCount)

        val stale = detectZygiskModule(sections, "boot_id=old", self, "boot-1") as ModuleState.Installed
        assertEquals(false, stale.active)

        val noHeartbeat = detectZygiskModule(sections, "", self, "boot-1") as ModuleState.Installed
        assertEquals(false, noHeartbeat.active)
    }

    @Test
    fun `zygisk not installed when prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectZygiskModule(emptyMap(), "boot_id=x", self, "x"))
    }

    @Test
    fun `ports active when iptables probe reports connected chains`() {
        val sections =
            mapOf(
                "ports_prop" to "version=0.6.3",
                "ports_chain" to "1",
                "ports_observers" to "com.browser\n",
            )
        val state = detectPortsModule(sections, self) as ModuleState.Installed
        assertEquals(true, state.active)
        assertEquals(1, state.targetCount)
    }

    @Test
    fun `ports inactive when iptables probe reports missing chains or jumps`() {
        val sections =
            mapOf(
                "ports_prop" to "version=0.6.3",
                "ports_chain" to "0",
                "ports_observers" to "com.browser\n",
            )
        val state = detectPortsModule(sections, self) as ModuleState.Installed
        assertEquals(false, state.active)
        assertEquals(1, state.targetCount)
    }

    @Test
    fun `ports apply problem uses current boot failure detail`() {
        val ports = ModuleState.Installed(version = "0.6.3", active = false, targetCount = 1)
        val problem =
            detectPortsApplyProblem(
                ports,
                "boot_id=boot-1\nloaded=0\ndetail=iptables-restore failed\n",
                currentBootId = "boot-1",
            )

        assertEquals("iptables-restore failed", problem?.failureDetail)
    }

    @Test
    fun `ports apply problem is generic when chains are missing without current boot failure`() {
        val ports = ModuleState.Installed(version = "0.6.3", active = false, targetCount = 1)

        assertEquals(null, detectPortsApplyProblem(ports.copy(active = true), "", "boot-1"))
        assertEquals(null, detectPortsApplyProblem(ports.copy(targetCount = 0), "", "boot-1"))
        assertEquals(
            PortsApplyProblem(failureDetail = null),
            detectPortsApplyProblem(ports, "boot_id=old\nloaded=0\ndetail=old failure\n", "boot-1"),
        )
    }

    @Test
    fun `ports not installed when prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectPortsModule(mapOf("ports_chain" to "1"), self))
    }
}
