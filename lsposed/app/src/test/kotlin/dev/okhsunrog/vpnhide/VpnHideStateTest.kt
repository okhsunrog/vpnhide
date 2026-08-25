package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.AppInfo
import dev.okhsunrog.vpnhide.debug.DeviceInfo
import dev.okhsunrog.vpnhide.debug.RootShellDiag
import dev.okhsunrog.vpnhide.debug.VpnHideState
import dev.okhsunrog.vpnhide.debug.redactSections
import dev.okhsunrog.vpnhide.debug.toJson
import dev.okhsunrog.vpnhide.diagnostics.CheckOutcome
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticReport
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The whole point of the refactor: the domain state serializes straight to the
 * debug JSON with no hand-written DTO. This pins that the full polymorphic
 * aggregate (sealed [ModuleState] / [LayerStatus] / [CheckOutcome] + [DiagnosticReport])
 * actually encodes at runtime — a broken @Serializable wiring throws only on encode,
 * which a compile check would miss.
 */
class VpnHideStateTest {
    private fun installed(
        active: Boolean,
        runtimeCheckable: Boolean = true,
    ) = ModuleState.Installed(version = "1.2.4", active = active, runtimeCheckable = runtimeCheckable)

    private fun sampleState(kmod: ModuleState): VpnHideState {
        val backends =
            NativeBackendStates(kmod = kmod, kpm = ModuleState.NotInstalled, zygisk = ModuleState.NotInstalled)
        val report =
            buildDiagnosticReport(
                gate = DiagnosticGate.ROUTED,
                results = CheckResults(native = emptyList()),
                backend = displayNativeBackend(backends),
                lsposedActive = true,
                complete = true,
            )
        return VpnHideState(
            generatedAt = "2026-08-19T13:00:00+0300",
            captureKind = "debug",
            app = AppInfo("dev.okhsunrog.vpnhide", "1.2.4 (10204)"),
            device = DeviceInfo("Google", "Pixel 8 Pro", "17", 36, listOf("arm64-v8a")),
            selfNeedsRestart = false,
            gate = report.gate,
            nativeVerdict = report.nativeVerdict,
            javaVerdict = report.javaVerdict,
            report = report,
            backends = backends,
            activeBackend = displayNativeBackend(backends),
            ports = installed(active = true),
            kmodLoadStatus = null,
            rootShell =
                RootShellDiag.from(
                    mapOf("snapshot_shell_uid" to "uid=0\nid=uid=0(root)\ncontext=u:r:ksu:s0\nerrno_ctl=ok"),
                ),
            sections = mapOf("proc_exists" to "1", "current_boot_id" to "boot-1"),
            dmesg = "vpnhide: loaded",
            logcat = "",
            bootLsposedLogcat = "",
            lsposedConfigDb = "",
            hookReport = null,
            debugCapture = null,
            errors = emptyList(),
        )
    }

    @Test
    fun `the full state aggregate serializes to JSON`() {
        val json = sampleState(installed(active = true)).toJson()
        // Compact sealed discriminators (from @SerialName), not fully-qualified names.
        assertTrue("module discriminator", json.contains("\"kind\": \"installed\""))
        assertTrue("module version carried", json.contains("\"1.2.4\""))
        assertTrue("root-shell self-diagnosis carried", json.contains("\"runtimeCheckable\""))
        assertTrue("raw sections preserved verbatim", json.contains("\"proc_exists\""))
        assertTrue("no fully-qualified class names leak into the wire", !json.contains("dev.okhsunrog.vpnhide.ModuleState"))
    }

    @Test
    fun `the dumped sections redact the user-identifying package and user lists`() {
        val raw =
            mapOf(
                "proc_exists" to "1",
                "pm_packages" to "package:/data/app/com.bank/base.apk=com.bank uid:10123",
                "pm_users" to "UserInfo{0:Alice:c13} running",
                "kmod_state" to "vpnhide 1 status",
            )
        val redacted = redactSections(raw)
        assertTrue("keeps non-identifying sections", redacted.containsKey("proc_exists"))
        assertTrue("keeps kmod_state", redacted.containsKey("kmod_state"))
        assertTrue("drops the installed-app list", !redacted.containsKey("pm_packages"))
        assertTrue("drops the profile list", !redacted.containsKey("pm_users"))
    }

    @Test
    fun `an unverified module serializes its runtime-unverified state`() {
        val json = sampleState(installed(active = false, runtimeCheckable = false)).toJson()
        assertTrue(json.contains("\"runtimeCheckable\": false"))
    }
}
