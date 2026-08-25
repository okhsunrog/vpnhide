package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.AppInfo
import dev.okhsunrog.vpnhide.debug.DeviceInfo
import dev.okhsunrog.vpnhide.debug.RootShellDiag
import dev.okhsunrog.vpnhide.debug.VPNHIDE_STATE_SCHEMA
import dev.okhsunrog.vpnhide.debug.VpnHideState
import dev.okhsunrog.vpnhide.debug.toJson
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticReport
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Pins the serialized shape of the debug bundle against a checked-in golden file.
 *
 * `state.json` is read by people (and agents) triaging bug reports, often months
 * after the app that wrote it shipped, using docs/debug-bundle.md as the map. A
 * field that silently renames itself — or a sealed `kind` that changes because a
 * class moved package — makes that doc quietly wrong and old bundles
 * un-diffable. Nothing else notices: the encoder happily emits whatever the
 * classes currently say.
 *
 * So the shape is frozen here. When this test fails, the change is real: refresh
 * the golden with `-DupdateGolden=true`, and if the change removes/renames a
 * field or alters its meaning, bump [VPNHIDE_STATE_SCHEMA] and add a row to the
 * history table in docs/debug-bundle.md. A pure addition needs no bump — only
 * the refreshed golden.
 */
class BundleSchemaGoldenTest {
    private val golden = File("src/test/resources/bundle/state_golden.json")

    @Test
    fun `the serialized bundle matches the golden file`() {
        val actual = sampleBundleState().toJson()

        // Env var, not a -D property: Gradle forwards the environment to the test
        // JVM but not its own system properties.
        if (System.getenv("UPDATE_GOLDEN") == "1") {
            golden.parentFile?.mkdirs()
            golden.writeText(actual)
        }

        assertTrue(
            "golden file missing — regenerate with: " +
                "UPDATE_GOLDEN=1 ./gradlew :app:testDebugUnitTest --tests '*BundleSchemaGoldenTest*'",
            golden.isFile,
        )
        assertEquals(
            "The debug bundle's serialized shape changed. If this is intended: refresh the golden with " +
                "UPDATE_GOLDEN=1, and bump VPNHIDE_STATE_SCHEMA + add a docs/debug-bundle.md history " +
                "row when a field was removed, renamed, or changed meaning.",
            golden.readText(),
            actual,
        )
    }

    @Test
    fun `no fully-qualified class name reaches the wire`() {
        // Sealed hierarchies without an explicit @SerialName serialize their
        // discriminator as the FQCN, which both bloats the bundle and pins the
        // classes to their current package. Catching it here keeps `kind` values
        // stable across any future package move.
        val json = sampleBundleState().toJson()
        assertTrue("FQCN leaked into the bundle:\n$json", !json.contains("dev.okhsunrog.vpnhide."))
    }

    @Test
    fun `the golden carries the current schema number`() {
        assertTrue(
            "golden not regenerated after a schema bump",
            golden.readText().contains("\"schema\": $VPNHIDE_STATE_SCHEMA"),
        )
    }
}

/**
 * A fixed, fully-populated state: every optional block filled, so the golden
 * covers the whole shape rather than the fields one capture path happens to set.
 * Values are constants — no clock, no device — so the encoding is reproducible.
 */
internal fun sampleBundleState(): VpnHideState {
    val kmod = ModuleState.Installed(version = "1.2.5", active = true, runtimeCheckable = true)
    val backends =
        NativeBackendStates(
            kmod = kmod,
            kpm = ModuleState.NotInstalled,
            zygisk = ModuleState.Installed(version = "1.2.5", active = false, runtimeCheckable = true),
        )
    val report =
        buildDiagnosticReport(
            gate = DiagnosticGate.ROUTED,
            results = CheckResults(native = emptyList()),
            backend = displayNativeBackend(backends),
            lsposedActive = true,
            complete = true,
        )
    return VpnHideState(
        generatedAt = "2026-08-22T12:00:00+0300",
        captureKind = "debug",
        app = AppInfo("dev.okhsunrog.vpnhide", "1.2.5 (10205)"),
        device = DeviceInfo("Google", "Pixel 8 Pro", "17", 36, listOf("arm64-v8a")),
        selfNeedsRestart = false,
        gate = report.gate,
        nativeVerdict = report.nativeVerdict,
        javaVerdict = report.javaVerdict,
        report = report,
        backends = backends,
        activeBackend = displayNativeBackend(backends),
        ports = ModuleState.NotInstalled,
        kmodLoadStatus = null,
        // The bridge-only block, included here on purpose: LsposedState and
        // ProtectionCheck are only reachable through it, and they are exactly the
        // sealed types whose discriminators used to be fully-qualified names.
        dashboard =
            DashboardState(
                kmod = kmod,
                kpm = ModuleState.NotInstalled,
                zygisk = backends.zygisk,
                lsposed = LsposedState.Active(version = "1.2.5", targetCount = 3),
                ports = ModuleState.NotInstalled,
                nativeTargetCount = 3,
                portsTargetCount = 0,
                nativeBackend = displayNativeBackend(backends),
                nativeInstallRecommendation = null,
                kmodLoadStatus = null,
                protection = ProtectionCheck.Blocked(DiagnosticGate.VPN_OFF),
                messages = emptyList(),
            ),
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
