package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Test

class DetectModulesTest {
    @Test
    fun `kmod not installed when prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectKmodModule(emptyMap()))
    }

    @Test
    fun `kmod active when proc node present`() {
        val sections =
            mapOf(
                "kmod_prop" to "version=0.6.3\ngkiVariant=android13-5.10",
                "proc_exists" to "1",
            )
        val state = detectKmodModule(sections) as ModuleState.Installed
        assertEquals("0.6.3", state.version)
        assertEquals(true, state.active)
        assertEquals("android13-5.10", state.gkiVariant)
    }

    @Test
    fun `kmod inactive when proc node absent`() {
        val sections = mapOf("kmod_prop" to "version=0.6.3", "proc_exists" to "0")
        assertEquals(false, (detectKmodModule(sections) as ModuleState.Installed).active)
    }

    // ── kmod ↔ builtin disambiguation over the shared /proc/vpnhide_ctl node ──
    // Both the .ko and the in-tree driver own /proc/vpnhide_ctl; the only signal
    // that tells them apart is the `backend 0x<n>` line in the control status
    // reply captured in kmod_state (0x0 = .ko, 0x4 = built-in).

    private fun ctlStatus(backendHex: String) = "vpnhide 1 status\nbackend 0x$backendHex\nkver 0x0\nhooks 0x3ff\nerror 0x0\n"

    @Test
    fun `parseCtlBackendId reads the backend id, or null when unread`() {
        assertEquals(4, parseCtlBackendId(mapOf("kmod_state" to ctlStatus("4"))))
        assertEquals(0, parseCtlBackendId(mapOf("kmod_state" to ctlStatus("0"))))
        assertEquals(null, parseCtlBackendId(emptyMap()))
        assertEquals(null, parseCtlBackendId(mapOf("kmod_state" to "")))
    }

    @Test
    fun `builtin active when the control node reports the built-in backend`() {
        val sections =
            mapOf(
                "builtin_prop" to "id=vpnhide_builtin\nversion=1.2.5",
                "kmod_prop" to "id=vpnhide_kmod\nversion=1.2.5",
                "proc_exists" to "1",
                "kmod_state" to ctlStatus("4"),
            )
        val builtin = detectBuiltinModule(sections) as ModuleState.Installed
        assertEquals(true, builtin.active)
        assertEquals("1.2.5", builtin.version)
        // The .ko must NOT also be reported active — this proc is the in-tree
        // driver's, not the module's, even though the kmod_prop is present.
        assertEquals(false, (detectKmodModule(sections) as ModuleState.Installed).active)
    }

    @Test
    fun `kmod stays active when the control node reports the kmod backend`() {
        val sections =
            mapOf(
                "kmod_prop" to "version=1.2.5",
                "proc_exists" to "1",
                "kmod_state" to ctlStatus("0"),
            )
        assertEquals(true, (detectKmodModule(sections) as ModuleState.Installed).active)
        // No builtin companion → not installed.
        assertEquals(ModuleState.NotInstalled, detectBuiltinModule(sections))
    }

    @Test
    fun `kmod stays active when the control backend id is unread`() {
        // Non-root snapshot: proc_exists is 1 but the 0600 node's status could not
        // be read, so the backend id is unknown. The historical proc-present rule
        // must still hold — a null id must never demote the kmod.
        val sections =
            mapOf(
                "kmod_prop" to "version=1.2.5",
                "proc_exists" to "1",
            )
        assertEquals(true, (detectKmodModule(sections) as ModuleState.Installed).active)
    }

    @Test
    fun `builtin inactive but verified when the control node reports the kmod backend`() {
        val sections =
            mapOf(
                "builtin_prop" to "id=vpnhide_builtin\nversion=1.2.5",
                "proc_exists" to "1",
                "kmod_state" to ctlStatus("0"),
            )
        val builtin = detectBuiltinModule(sections) as ModuleState.Installed
        assertEquals(false, builtin.active)
        // proc/status were readable (root-equivalent snapshot, no uid probe → trusted).
        assertEquals(true, builtin.runtimeCheckable)
    }

    @Test
    fun `builtin runtime unverified when snapshot shell lacked root`() {
        // The 0600 /proc/vpnhide_ctl is EACCES to a non-root shell, so the id
        // can't be read and proc_exists reads 0 — not proof the backend is off.
        val sections =
            mapOf(
                "builtin_prop" to "id=vpnhide_builtin\nversion=1.2.5",
                "proc_exists" to "0",
                "snapshot_shell_uid" to "uid=2000\nid=uid=2000(shell)\ncontext=u:r:shell:s0",
            )
        val builtin = detectBuiltinModule(sections) as ModuleState.Installed
        assertEquals(false, builtin.active)
        assertEquals(false, builtin.runtimeCheckable)
    }

    @Test
    fun `builtin not installed when its companion prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectBuiltinModule(mapOf("proc_exists" to "1", "kmod_state" to ctlStatus("4"))))
    }

    @Test
    fun `kmod runtime unverified when snapshot shell lacked root`() {
        // proc_exists reads 0 because the 0600 /proc/vpnhide_ctl was EACCES to a
        // non-root shell — NOT because the module is inactive. Mark it unverified.
        val sections =
            mapOf(
                "kmod_prop" to "version=1.2.3",
                "proc_exists" to "0",
                "snapshot_shell_uid" to "uid=2000\nid=uid=2000(shell)\ncontext=u:r:shell:s0\nerrno_ctl=eacces",
            )
        val state = detectKmodModule(sections) as ModuleState.Installed
        assertEquals(false, state.active)
        assertEquals(false, state.runtimeCheckable)
    }

    @Test
    fun `kmod genuinely inactive when a root shell confirms the proc node is absent`() {
        val sections =
            mapOf(
                "kmod_prop" to "version=1.2.3",
                "proc_exists" to "0",
                "snapshot_shell_uid" to "uid=0\nid=uid=0(root)\ncontext=u:r:ksu:s0\nerrno_ctl=enoent",
            )
        val state = detectKmodModule(sections) as ModuleState.Installed
        assertEquals(false, state.active)
        assertEquals(true, state.runtimeCheckable)
    }

    @Test
    fun `ports runtime unverified when snapshot shell lacked root`() {
        val sections =
            mapOf(
                "ports_prop" to "version=1.2.3",
                "ports_chain" to "0",
                "snapshot_shell_uid" to "uid=2000\nid=uid=2000(shell)\ncontext=u:r:shell:s0",
            )
        val state = detectPortsModule(sections) as ModuleState.Installed
        assertEquals(false, state.active)
        assertEquals(false, state.runtimeCheckable)
    }

    @Test
    fun `zygisk active only when heartbeat matches current boot`() {
        val sections = mapOf("zygisk_prop" to "version=0.6.3")
        val fresh = detectZygiskModule(sections, "boot_id=boot-1", "boot-1") as ModuleState.Installed
        assertEquals(true, fresh.active)

        val stale = detectZygiskModule(sections, "boot_id=old", "boot-1") as ModuleState.Installed
        assertEquals(false, stale.active)

        val noHeartbeat = detectZygiskModule(sections, "", "boot-1") as ModuleState.Installed
        assertEquals(false, noHeartbeat.active)
    }

    @Test
    fun `zygisk not installed when prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectZygiskModule(emptyMap(), "boot_id=x", "x"))
    }

    @Test
    fun `ports active when iptables probe reports connected chains`() {
        val sections =
            mapOf(
                "ports_prop" to "version=0.6.3",
                "ports_chain" to "1",
            )
        val state = detectPortsModule(sections) as ModuleState.Installed
        assertEquals(true, state.active)
    }

    @Test
    fun `ports inactive when iptables probe reports missing chains or jumps`() {
        val sections =
            mapOf(
                "ports_prop" to "version=0.6.3",
                "ports_chain" to "0",
            )
        val state = detectPortsModule(sections) as ModuleState.Installed
        assertEquals(false, state.active)
    }

    @Test
    fun `ports apply problem uses current boot failure detail`() {
        val ports = ModuleState.Installed(version = "0.6.3", active = false)
        val problem =
            detectPortsApplyProblem(
                ports,
                1,
                "boot_id=boot-1\nloaded=0\ndetail=iptables-restore failed\n",
                currentBootId = "boot-1",
                portsDisabled = false,
            )

        assertEquals("iptables-restore failed", problem?.failureDetail)
    }

    @Test
    fun `ports apply problem is generic when chains are missing without current boot failure`() {
        val ports = ModuleState.Installed(version = "0.6.3", active = false)

        assertEquals(null, detectPortsApplyProblem(ports.copy(active = true), 1, "", "boot-1", portsDisabled = false))
        assertEquals(null, detectPortsApplyProblem(ports, 0, "", "boot-1", portsDisabled = false))
        assertEquals(
            PortsApplyProblem(failureDetail = null),
            detectPortsApplyProblem(ports, 1, "boot_id=old\nloaded=0\ndetail=old failure\n", "boot-1", portsDisabled = false),
        )
    }

    @Test
    fun `ports apply problem is suppressed for a deliberately disabled module`() {
        val ports = ModuleState.Installed(version = "0.6.3", active = false)

        assertEquals(
            null,
            detectPortsApplyProblem(
                ports,
                1,
                "boot_id=boot-1\nloaded=0\ndetail=iptables-restore failed\n",
                currentBootId = "boot-1",
                portsDisabled = true,
            ),
        )
    }

    @Test
    fun `ports apply problem does not duplicate a bundle integrity failure`() {
        val ports =
            ModuleState.Installed(
                version = "0.6.3",
                active = false,
                brokenReason = ModuleBrokenReason.ActivatorMissing,
            )

        assertEquals(null, detectPortsApplyProblem(ports, 1, "", "boot-1", portsDisabled = false))
    }

    @Test
    fun `ports not installed when prop is absent`() {
        assertEquals(ModuleState.NotInstalled, detectPortsModule(mapOf("ports_chain" to "1")))
    }
}
