package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.AppInfo
import dev.okhsunrog.vpnhide.debug.BeforeCaptureInfo
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
 * the golden with `UPDATE_GOLDEN=1`, bump [VPNHIDE_STATE_SCHEMA] for the exported
 * serializable model change and add a row to the history table in
 * docs/debug-bundle.md, following the module's AGENTS.md contract.
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
    return VpnHideState(
        generatedAt = "2026-08-22T12:00:00+0300",
        captureKind = "debug",
        app = AppInfo("dev.okhsunrog.vpnhide", "1.2.5 (10205)"),
        device = DeviceInfo("Google", "Pixel 8 Pro", "17", 36, listOf("arm64-v8a")),
        selfNeedsRestart = false,
        gate = null,
        selfTestRunId = null,
        diagnostics = null,
        autonomousDiagnostics = sampleAutonomousDiagnosticInfo(),
        nativeStatus = sampleNativeStatus(boot = sampleBoot()),
        nativeVerdict = null,
        javaVerdict = null,
        report = null,
        backends = backends,
        activeBackend = DisplayNativeBackend(NativeBackendId.Kmod, kmod),
        ports = ModuleState.NotInstalled,
        kmodLoadStatus = null,
        dashboard = sampleDashboard(kmod, backends),
        beforeCapture = sampleBeforeCapture(),
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
        nativeBackend = DisplayNativeBackend(NativeBackendId.Kmod, kmod),
        protection = ProtectionCheck.Autonomous(sampleAutonomousDiagnosticInfo()),
        messages = emptyList(),
    )

private fun sampleBeforeCapture(): BeforeCaptureInfo =
    BeforeCaptureInfo(
        observedAt = "2026-08-22T11:59:59+0300",
        diagnostics = null,
        autonomousDiagnostics = sampleAutonomousDiagnosticInfo().copy(current = false),
        protection = null,
        messages =
            listOf(
                DashboardMessage(
                    DashboardMessageSeverity.WARNING,
                    "Update module",
                    reason =
                        DashboardIssue
                            .CoreVersionMismatch(
                                "1.2.0",
                                "1.3.0",
                                null,
                            ).toReason(),
                    presentationLanguage = "en",
                ),
            ),
        pendingOperationIds = listOf(7),
        operationRechecking = true,
    )

internal fun sampleAutonomousDiagnosticInfo(): dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticInfo {
    val required = sampleRequiredChecks()
    val proven = listOf("ioctl_flags", "ioctl_mtu", "getifaddrs", "netlink_links", "framework_capabilities")
    val leaked = listOf("framework_networks")
    return dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticInfo(
        availability = "published",
        current = true,
        sourceEpoch = 9,
        sourceConfigRevision = 3,
        sourceSequence = 12,
        connectionGeneration = 1,
        snapshotRevision = 14,
        rootObservationId = 9,
        rootGeneration = 3,
        rootJoined = true,
        rootSourceEpoch = 9,
        rootSourceSequence = 12,
        rootSourceConnectionGeneration = 1,
        rootSourceConfigRevision = 3,
        selectedBackend = "kmod",
        appliedBackend = "kmod",
        mutationHeld = false,
        heldResources = emptyList(),
        run = sampleAutonomousRun(),
        assessment =
            dev.okhsunrog.vpnhide.diagnostics.AutonomousAssessmentInfo(
                phase = "incomplete",
                required = required,
                proven = proven,
                leaked = leaked,
                unverified = required - proven.toSet() - leaked.toSet(),
                nativeControls = emptyList(),
                frameworkControls = listOf("probe_unavailable"),
                reuseBlockers = listOf("loaded_generations_incomplete"),
                infrastructureDetail = null,
                cleanupConfirmed = null,
            ),
    )
}

private fun sampleRequiredChecks() =
    listOf(
        "ioctl_flags",
        "ioctl_mtu",
        "getifaddrs",
        "netlink_links",
        "ioctl_addresses",
        "bind_to_device",
        "netlink_addresses",
        "netlink_routes",
        "netlink_rules",
        "proc_routes",
        "proc_interfaces",
        "sys_interfaces",
        "framework_networks",
        "framework_default_network",
        "framework_capabilities",
        "framework_link_properties",
        "framework_legacy_info",
        "framework_callbacks",
        "framework_pending_intent",
    )

private fun sampleFrameworkAttribution() =
    dev.okhsunrog.vpnhide.diagnostics.AutonomousFrameworkAttributionInfo(
        99,
        9,
        "boot-1",
        10041,
        2345,
        15000,
        100,
        320,
        "c".repeat(64),
        "d".repeat(64),
    )

private fun sampleAutonomousRun() =
    dev.okhsunrog.vpnhide.diagnostics.AutonomousRunInfo(
        session = 99,
        coreEpoch = 9,
        bootId = "boot-1",
        policySha256 = "a".repeat(64),
        runtimeSha256 = "b".repeat(64),
        platformSha256 = "e".repeat(64),
        loadedGenerationsSha256 = "f".repeat(64),
        loadedGenerationsComplete = false,
        contract = 3,
        stale = false,
        frameworkAttribution =
            sampleFrameworkAttribution(),
    )

private fun sampleBoot() =
    NativeBootStatus(
        bootId = "boot-1",
        decidedAtMs = 1,
        outcome = NativeBootOutcome.Selected(NativeBackendId.Builtin, NativeSelectionMode.Adopted, false),
        installed = emptyList(),
        live = listOf(NativeBackendId.Builtin),
        staged = emptyList(),
        kmod = null,
        zygiskTargets = ZygiskTargetsState.NotInstalled,
        ports = PortsBootStatus(PortsBootSource.Boot, FamilyOutcome.Confirmed, FamilyOutcome.Confirmed, 0, true, "", 1),
    )
