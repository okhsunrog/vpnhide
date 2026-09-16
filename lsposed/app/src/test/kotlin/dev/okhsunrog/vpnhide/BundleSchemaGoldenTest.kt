package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.AppInfo
import dev.okhsunrog.vpnhide.debug.CallbackEvent
import dev.okhsunrog.vpnhide.debug.CapabilityFacts
import dev.okhsunrog.vpnhide.debug.DeviceInfo
import dev.okhsunrog.vpnhide.debug.InvariantResult
import dev.okhsunrog.vpnhide.debug.LegacyTypeView
import dev.okhsunrog.vpnhide.debug.LinkFacts
import dev.okhsunrog.vpnhide.debug.NET_VIEW_NA
import dev.okhsunrog.vpnhide.debug.NET_VIEW_OK
import dev.okhsunrog.vpnhide.debug.NET_VIEW_VIOLATED
import dev.okhsunrog.vpnhide.debug.NetworkFacts
import dev.okhsunrog.vpnhide.debug.NetworkInfoFacts
import dev.okhsunrog.vpnhide.debug.NetworkViewSnapshot
import dev.okhsunrog.vpnhide.debug.PendingIntentResult
import dev.okhsunrog.vpnhide.debug.RootShellDiag
import dev.okhsunrog.vpnhide.debug.VPNHIDE_STATE_SCHEMA
import dev.okhsunrog.vpnhide.debug.VpnHideState
import dev.okhsunrog.vpnhide.debug.toJson
import dev.okhsunrog.vpnhide.diagnostics.CheckResults
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticAttemptInfo
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticEligibility
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticMeasurementInfo
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticStage
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticSummaryInfo
import dev.okhsunrog.vpnhide.diagnostics.EvidenceConclusion
import dev.okhsunrog.vpnhide.diagnostics.MeasurementApplicability
import dev.okhsunrog.vpnhide.diagnostics.MeasurementEvidence
import dev.okhsunrog.vpnhide.diagnostics.RunOutcome
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
        selfTestRunId = 4,
        diagnostics = sampleDiagnosticSummary(),
        nativeVerdict = report.nativeVerdict,
        javaVerdict = report.javaVerdict,
        report = report,
        backends = backends,
        activeBackend = displayNativeBackend(backends),
        ports = ModuleState.NotInstalled,
        kmodLoadStatus = null,
        dashboard = sampleDashboard(kmod, backends),
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
        networkView = sampleNetworkView(),
        debugCapture = null,
        errors = emptyList(),
    )
}

/** One network, one callback, one invariant of each status: pins every nested shape. */
private fun sampleNetworkView(): NetworkViewSnapshot {
    val caps = CapabilityFacts(listOf("WIFI"), listOf("INTERNET", "NOT_VPN", "VALIDATED"), "WifiInfo", -1)
    val lp = LinkFacts("wlan0", listOf("10.0.0.2/24"), listOf("10.0.0.1"), listOf("0.0.0.0/0 via 10.0.0.1 dev wlan0"), emptyList(), 1500)
    val info = NetworkInfoFacts(1, "WIFI", 0, "CONNECTED", "CONNECTED", true, null)
    return NetworkViewSnapshot(
        uid = 10332,
        packageName = "dev.okhsunrog.vpnhide",
        sdk = 36,
        capturedAt = "2026-08-22T12:00:05+0300",
        captureMs = 2000,
        expectHidden = true,
        activeNetwork = 100,
        boundNetwork = null,
        allNetworks = listOf(100),
        networks = listOf(NetworkFacts(100, listOf("all", "active", "callback"), caps, lp, info)),
        legacy =
            LegacyTypeView(
                activeNetworkInfo = info,
                byType = mapOf("VPN" to NetworkInfoFacts(17, "VPN", 0, "DISCONNECTED", "DISCONNECTED", true, null), "WIFI" to info),
                allNetworkInfo = listOf(info),
                networkForType = mapOf("VPN" to null, "WIFI" to 100),
            ),
        phantomNetworks = emptyList(),
        callbacks = listOf(CallbackEvent("default", "capabilities", 100, 12, capabilities = caps)),
        pendingIntent = PendingIntentResult(registered = true, netIds = listOf(100), requestNetIds = listOf(100)),
        invariants =
            listOf(
                InvariantResult("active_in_all_networks", NET_VIEW_OK, "active=100 listed"),
                InvariantResult("no_phantom_networks", NET_VIEW_VIOLATED, "answering netIds [103]"),
                InvariantResult("pending_intent_only_listed_networks", NET_VIEW_NA, "not captured"),
            ),
        errors = emptyList(),
    )
}

/**
 * The bridge-only block, included in the golden on purpose: LsposedState and
 * ProtectionCheck are only reachable through it, and they are exactly the sealed
 * types whose discriminators used to be fully-qualified names.
 */
private fun sampleDashboard(
    kmod: ModuleState,
    backends: NativeBackendStates,
): DashboardState =
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
    )

/**
 * The presentation block with every optional field set: a run in flight, a
 * failed later attempt and an older complete measurement whose conditions
 * changed since, so the golden pins each nested shape and every enum wire name.
 */
private fun sampleDiagnosticSummary(): DiagnosticSummaryInfo =
    DiagnosticSummaryInfo(
        eligibility = DiagnosticEligibility.Eligible,
        activeRunId = 5,
        activeStage = DiagnosticStage.Core,
        lastAttempt = DiagnosticAttemptInfo(4, RunOutcome.Failed, TransitionFailure.ExecutionFailed, eligibility = null),
        measurement =
            DiagnosticMeasurementInfo(
                runId = 3,
                startedAt = 1_755_856_800_000,
                endedAt = 1_755_856_802_000,
                completed = true,
                interrupted = false,
                observationId = 7,
            ),
        applicability = MeasurementApplicability.Changed,
        evidence =
            MeasurementEvidence(
                hidden = 9,
                systemBlocked = 1,
                nothingToLeak = 2,
                leaks = 0,
                notMeasured = 0,
                notRun = 0,
                uncoveredLeaks = 0,
                conclusion = EvidenceConclusion.NoObservedLeak,
            ),
        currentSuccess = false,
        probeUnavailable = true,
    )
