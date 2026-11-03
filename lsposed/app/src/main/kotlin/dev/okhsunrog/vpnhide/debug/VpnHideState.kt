package dev.okhsunrog.vpnhide.debug

import android.content.Context
import android.os.Build
import dev.okhsunrog.vpnhide.AgentStatisticsState
import dev.okhsunrog.vpnhide.DashboardMessage
import dev.okhsunrog.vpnhide.DashboardState
import dev.okhsunrog.vpnhide.DebugShellSnapshot
import dev.okhsunrog.vpnhide.DisplayNativeBackend
import dev.okhsunrog.vpnhide.LsposedState
import dev.okhsunrog.vpnhide.ModuleState
import dev.okhsunrog.vpnhide.NativeBackendStates
import dev.okhsunrog.vpnhide.ProtectionCheck
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.coreDisplayNativeBackend
import dev.okhsunrog.vpnhide.detectPortsModule
import dev.okhsunrog.vpnhide.diagnostics.toDiagnosticInfo
import dev.okhsunrog.vpnhide.hooksInMask
import dev.okhsunrog.vpnhide.lsposedHooksActiveThisBoot
import dev.okhsunrog.vpnhide.nativeLegacyInventory
import dev.okhsunrog.vpnhide.nativeStatusSnapshot
import dev.okhsunrog.vpnhide.snapshotRuntimeCheckable
import dev.okhsunrog.vpnhide.snapshotShellUid
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement

/**
 * The one canonical, fully-serializable app-state snapshot.
 *
 * Retained Core diagnostic evidence, native state, presentation and optional
 * forensic attachments share one serializable snapshot. Capturing it does not
 * admit a diagnostic run. Capture time and each source's measurement identity
 * remain separate; legacy nullable output slots do not recreate removed owners.
 *
 * Nothing machine-parses this format but an operator (or an AI) reading a bug
 * report, so the shape may change freely — but a bundle can arrive months late,
 * from an app version nobody has the source of at hand, so the version it was
 * written against travels with it.
 *
 * The number below is the ONE version of the whole bundle (there is no separate
 * per-object version). Bump it when an exported serializable model changes.
 * `BundleSchemaGoldenTest`
 * fails until the golden file is updated, so no shape change ships unnoticed.
 * Every version gets a row in the history table in docs/debug-bundle.md — the
 * number is only worth carrying if it can be looked up.
 *
 * 2: sealed `kind` discriminators are compact snake_case everywhere. Before
 *    this, LsposedState and ProtectionCheck serialized as fully-qualified class
 *    names, which also made them unmovable between packages.
 * 1: initial.
 */
internal const val VPNHIDE_STATE_SCHEMA: Int = 10

/**
 * What to include in a captured [VpnHideState]. The ONE source of truth for the
 * export "recipe": the agent-bridge getState args decode straight into it, the
 * file-export modal's toggles produce it, and [buildVpnHideState] consumes it —
 * so a switch on screen, a bridge argument and a builder branch can never drift.
 *
 * These are CONTENT options (they shape the JSON). Attaching a kernel image is a
 * packaging choice (JSON → zip, binary), not a content option, so it lives only on
 * the file-export path, never here.
 */
@Serializable
internal data class StateContentOptions(
    // dmesg, logcat, boot logcat, lsposed config, hook report, and the raw shell
    // sections. Off by default → a small, quick state payload.
    val forensics: Boolean = false,
    // The installed-app list + profile names (pm_packages/pm_users). Off by default
    // for privacy; opt in — with the user's explicit consent on the modal — only
    // when the developer needs to see which apps are present. No effect without
    // [forensics] (the lists live in the raw sections).
    val appList: Boolean = false,
)

@Serializable
internal data class VpnHideState(
    val schema: Int = VPNHIDE_STATE_SCHEMA,
    // ISO-8601, stamped by the caller (the serializer has no clock).
    val generatedAt: String,
    // "debug" | "full_system_logcat" | "kernel_images"
    val captureKind: String,
    val app: AppInfo,
    val device: DeviceInfo,
    val selfNeedsRestart: Boolean,
    // Legacy compatibility slots, always null for current Core-backed captures.
    val gate: JsonElement?,
    val selfTestRunId: Long? = null,
    val diagnostics: JsonElement? = null,
    val nativeStatus: dev.okhsunrog.vpnhide.NativeStatusSnapshot? = null,
    val autonomousDiagnostics: dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticInfo? = null,
    // Legacy compatibility slots; current assessment belongs to autonomousDiagnostics.
    val nativeVerdict: JsonElement?,
    val javaVerdict: JsonElement?,
    val report: JsonElement?,
    // Per-module state, exactly as the detectors compute it for the dashboard.
    val backends: NativeBackendStates,
    val activeBackend: DisplayNativeBackend,
    val ports: ModuleState,
    val kmodLoadStatus: JsonElement? = null,
    // Dashboard derived from the same root snapshot and diagnostic presentation.
    // Null if assembly failed (errors explains why), or for captures that omit it.
    val dashboard: DashboardState? = null,
    val beforeCapture: BeforeCaptureInfo? = null,
    // Desired-state config embedded as structured JSON (the canonical
    // /data/system/vpnhide_config.json). Lets one read answer "is app X even a
    // target" alongside the runtime state. Null when not requested.
    val config: JsonElement? = null,
    // Current hook-counter snapshot. The stateful baseline/diff capture stays a
    // separate bridge call; this is just the point-in-time totals.
    val statistics: AgentStatisticsState? = null,
    // Root-shell self-diagnosis: who the snapshot shell ran as, and whether its
    // liveness probes are trustworthy. This is what tells a "not verified" module
    // apart from a genuinely inactive one.
    val rootShell: RootShellDiag,
    // Every raw shell-probe section verbatim — the ground truth the typed fields
    // above were derived from, plus all forensic captures (network, proc/net,
    // kernel symbols, module inventory, …). Lossless.
    val sections: Map<String, String>,
    val dmesg: String,
    val logcat: String,
    val bootLsposedLogcat: String,
    val lsposedConfigDb: String,
    // Hook installation/counter forensics over the capture interval. Collection
    // does not admit a diagnostic run. Null when no baseline was captured.
    val hookReport: String?,
    // The framework network model as this (target) uid sees it after Binder —
    // handles, per-handle facts, legacy answers, callback deliveries — and the
    // consistency invariants over them (see NetworkViewData.kt). Forensics only;
    // null when the capture could not take it (the reason is in [errors]).
    val networkView: NetworkViewSnapshot? = null,
    val debugCapture: DebugCaptureInfo?,
    // Self-documenting: the options this capture was built with (what was included).
    val captureOptions: StateContentOptions = StateContentOptions(),
    // Non-fatal capture failures (a probe that threw, a truncated section). The
    // document always serializes; partial data is flagged here rather than lost.
    val errors: List<String> = emptyList(),
)

/** Available process state before logging or probes; never initiates a read. */
@Serializable
internal data class BeforeCaptureInfo(
    val observedAt: String,
    val diagnostics: JsonElement?,
    val autonomousDiagnostics: dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticInfo? = null,
    val protection: ProtectionCheck?,
    val messages: List<DashboardMessage>,
    val pendingOperationIds: List<Long>,
    val operationRechecking: Boolean,
    val lastOperationId: Long? = null,
    val lastOperationPhases: Map<String, String> = emptyMap(),
    val lastOperationFailure: String? = null,
)

@Serializable
internal data class AppInfo(
    val packageName: String,
    val version: String,
)

@Serializable
internal data class DeviceInfo(
    val manufacturer: String,
    val model: String,
    val androidRelease: String,
    val sdk: Int,
    val abis: List<String>,
)

@Serializable
internal data class DebugCaptureInfo(
    val forced: Boolean,
    val applyExit: Int?,
    val restoreExit: Int?,
    val detail: String?,
)

/**
 * The snapshot shell's identity + liveness-probe trust. Parsed from the
 * `snapshot_shell_uid` probe. When [uid] != 0 the shell lacked the privilege to
 * read the 0600 `/proc/vpnhide_ctl` or run iptables, so a negative liveness read
 * is unreliable and [runtimeCheckable] is false.
 */
@Serializable
internal data class RootShellDiag(
    val uid: Int?,
    val idLine: String,
    val context: String,
    // "ok" | "eacces" | "enoent" | "other:<msg>"
    val errnoCtl: String,
    val runtimeCheckable: Boolean,
) {
    companion object {
        fun from(sections: Map<String, String>): RootShellDiag {
            val raw = sections["snapshot_shell_uid"].orEmpty()

            fun line(prefix: String): String =
                raw
                    .lineSequence()
                    .firstOrNull { it.startsWith(prefix) }
                    ?.substringAfter(prefix)
                    ?.trim()
                    .orEmpty()
            return RootShellDiag(
                uid = snapshotShellUid(sections),
                idLine = line("id="),
                context = line("context="),
                errnoCtl = line("errno_ctl=").ifEmpty { "unknown" },
                runtimeCheckable = snapshotRuntimeCheckable(sections),
            )
        }
    }
}

private val stateJson =
    Json {
        prettyPrint = true
        encodeDefaults = true
        // Sealed-type discriminator; "kind" avoids colliding with any real "type"
        // key that might live inside a raw shell section rendered as a string map.
        classDiscriminator = "kind"
    }

internal fun VpnHideState.toJson(): String = stateJson.encodeToString(this)

// Raw shell sections that identify the user (full installed-app list with paths +
// UIDs; profile names) — omitted from the dumped `sections` the way the old text
// bundle deliberately never wrote them. The privacy-safe `app_scan_diagnostics`
// (redacted names, counts only) stays; the full list is available on explicit
// request via the agent-bridge listInstalledApps call.
private val REDACTED_SECTIONS = setOf("pm_packages", "pm_users")

/** Drop user-identifying sections from a dumped section map. */
internal fun redactSections(sections: Map<String, String>): Map<String, String> = sections - REDACTED_SECTIONS

/**
 * Fold one capture into the canonical [VpnHideState]. The module/liveness state is
 * derived from [rootSnapshot] — the SAME snapshot the live dashboard uses — so the
 * dump can never disagree with the on-screen state the way the old separate-shell
 * export path did. [shellSnapshot] contributes forensic-only sections (network,
 * proc/net, kernel symbols). Both section maps are preserved verbatim.
 *
 * Pure and non-suspending: every input is captured by the caller. Retained Core
 * evidence keeps its own source/run identity; collecting or packaging this state
 * does not run checks. Historical report/gate/verdict slots remain null.
 */
@Suppress("LongParameterList", "LongMethod")
internal fun buildVpnHideState(
    context: Context,
    captureKind: String,
    generatedAt: String,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
    shellSnapshot: DebugShellSnapshot?,
    autonomousSnapshot: dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticSnapshot,
    dmesg: String,
    logcat: String,
    bootLsposedLogcat: String,
    lsposedConfigDb: String,
    hookReport: String?,
    debugCapture: DebugCaptureInfo?,
    errors: List<String>,
    networkView: NetworkViewSnapshot? = null,
    dashboard: DashboardState? = null,
    beforeCapture: BeforeCaptureInfo? = null,
    statistics: AgentStatisticsState? = null,
    options: StateContentOptions = StateContentOptions(),
): VpnHideState {
    val rootSections = rootSnapshot.sections
    val nativeStatus = autonomousSnapshot.sources.core.nativeStatusSnapshot()
    val rootCurrent = autonomousSnapshot.rootJoined
    val observedBackends = nativeLegacyInventory(rootSections)
    val backends =
        if (rootCurrent) {
            observedBackends
        } else {
            observedBackends.copy(
                kmod = unverifiedExportRuntime(observedBackends.kmod),
                builtin = unverifiedExportRuntime(observedBackends.builtin),
                kpm = unverifiedExportRuntime(observedBackends.kpm),
                zygisk = unverifiedExportRuntime(observedBackends.zygisk),
            )
        }
    val activeBackend = coreDisplayNativeBackend(backends, nativeStatus)
    val observedPorts = detectPortsModule(rootSections)
    val ports = if (rootCurrent) observedPorts else unverifiedExportRuntime(observedPorts)
    return VpnHideState(
        generatedAt = generatedAt,
        captureKind = captureKind,
        app = AppInfo(context.packageName, appVersionText(context)),
        device =
            DeviceInfo(
                manufacturer = Build.MANUFACTURER,
                model = Build.MODEL,
                androidRelease = Build.VERSION.RELEASE.orEmpty(),
                sdk = Build.VERSION.SDK_INT,
                abis = Build.SUPPORTED_ABIS.orEmpty().toList(),
            ),
        selfNeedsRestart = selfNeedsRestart,
        gate = null,
        selfTestRunId = null,
        diagnostics = null,
        autonomousDiagnostics = autonomousSnapshot.toDiagnosticInfo(),
        nativeStatus = nativeStatus,
        nativeVerdict = null,
        javaVerdict = null,
        report = null,
        backends = backends,
        activeBackend = activeBackend,
        ports = ports,
        dashboard = dashboard,
        beforeCapture = beforeCapture,
        // Explicit config (the bridge lean call) wins; otherwise derive it from the
        // raw canonical-config section so the file export carries it structured too.
        config = autonomousExportCanonical(autonomousSnapshot),
        statistics = statistics.takeIf { rootCurrent },
        rootShell = RootShellDiag.from(rootSections),
        // Raw sections only in a forensic capture. Forensic (shell) sections on top of
        // the authoritative root sections; root wins on key collisions. The
        // user-identifying installed-app / profile lists are redacted unless the user
        // explicitly opted in via [StateContentOptions.appList].
        sections =
            if (options.forensics) {
                val merged = shellSnapshot?.sections.orEmpty() + rootSections
                if (options.appList) merged else redactSections(merged)
            } else {
                emptyMap()
            },
        dmesg = dmesg,
        logcat = logcat,
        bootLsposedLogcat = bootLsposedLogcat,
        lsposedConfigDb = lsposedConfigDb,
        hookReport = hookReport,
        networkView = networkView,
        debugCapture = debugCapture,
        captureOptions = options,
        errors = errors,
    )
}
