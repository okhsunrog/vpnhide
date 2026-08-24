package dev.okhsunrog.vpnhide

import android.content.Context
import android.os.Build
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement

/**
 * The one canonical, fully-serializable app-state snapshot.
 *
 * This is the whole point of the debug pipeline: the exact domain objects the UI
 * renders (the [DiagnosticReport], the per-module [ModuleState]s, the active
 * backend) are @Serializable, so the debug dump is `Json.encodeToString(state)` —
 * complete by construction, never a hand-written text projection that drifts out
 * of sync with what the app shows. Everything a bug report needs travels in one
 * object: the derived state, the raw shell sections it was derived from, the
 * captured logs, and the root-shell self-diagnosis.
 *
 * Nothing machine-parses this format but an operator (or an AI) reading a bug
 * report, so the shape may change freely — but a bundle can arrive months late,
 * from an app version nobody has the source of at hand, so the version it was
 * written against travels with it.
 *
 * The number below is the ONE version of the whole bundle (there is no separate
 * per-object version). Bump it when a field is removed, renamed, or changes
 * meaning; a pure addition needs no bump. Either way `BundleSchemaGoldenTest`
 * fails until the golden file is updated, so no shape change ships unnoticed.
 * Every version gets a row in the history table in docs/debug-bundle.md — the
 * number is only worth carrying if it can be looked up.
 *
 * 2: sealed `kind` discriminators are compact snake_case everywhere. Before
 *    this, LsposedState and ProtectionCheck serialized as fully-qualified class
 *    names, which also made them unmovable between packages.
 * 1: initial.
 */
internal const val VPNHIDE_STATE_SCHEMA: Int = 2

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
    // The measured diagnostics run. Null when the capture did not run checks
    // (e.g. the full-logcat recorder just bundles state, no probe run).
    val gate: DiagnosticGate?,
    // Verdicts are gate-checked getters on the report (not stored fields), so they
    // would not otherwise serialize — surface them explicitly, computed once here.
    val nativeVerdict: Verdict?,
    val javaVerdict: Verdict?,
    val report: DiagnosticReport?,
    // Per-module state, exactly as the detectors compute it for the dashboard.
    val backends: NativeBackendStates,
    val activeBackend: DisplayNativeBackend,
    val ports: ModuleState,
    val kmodLoadStatus: KmodLoadStatus?,
    // The full live dashboard model (hero/messages/recommendations on top of the
    // module states above). Populated by the agent-bridge getState (which has a
    // live DashboardState); null in the file export, which carries only the cheap
    // detector-derived fields above.
    val dashboard: DashboardState? = null,
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
    // Hook install/counter diagnostics (installed-hook mask, counter deltas across
    // the forced check run). Null for captures that don't take a counter baseline.
    val hookReport: String?,
    val debugCapture: DebugCaptureInfo?,
    // Self-documenting: the options this capture was built with (what was included).
    val captureOptions: StateContentOptions = StateContentOptions(),
    // Non-fatal capture failures (a probe that threw, a truncated section). The
    // document always serializes; partial data is flagged here rather than lost.
    val errors: List<String> = emptyList(),
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

/** Parse the raw canonical-config section into structured JSON, or null if absent/unparseable. */
private fun parseCanonicalConfigSection(raw: String?): JsonElement? =
    raw?.takeIf { it.isNotBlank() }?.let { runCatching { Json.parseToJsonElement(it) }.getOrNull() }

/**
 * Fold one capture into the canonical [VpnHideState]. The module/liveness state is
 * derived from [rootSnapshot] — the SAME snapshot the live dashboard uses — so the
 * dump can never disagree with the on-screen state the way the old separate-shell
 * export path did. [shellSnapshot] contributes forensic-only sections (network,
 * proc/net, kernel symbols). Both section maps are preserved verbatim.
 *
 * Pure and non-suspending: every input is captured by the caller. [gate] +
 * [checkResults] are non-null only for a real diagnostics run (the debug export);
 * the logcat/kernel captures pass null and carry no [report].
 */
@Suppress("LongParameterList", "LongMethod")
internal fun buildVpnHideState(
    context: Context,
    captureKind: String,
    generatedAt: String,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
    shellSnapshot: DebugShellSnapshot?,
    gate: DiagnosticGate?,
    checkResults: CheckResults?,
    dmesg: String,
    logcat: String,
    bootLsposedLogcat: String,
    lsposedConfigDb: String,
    hookReport: String?,
    debugCapture: DebugCaptureInfo?,
    errors: List<String>,
    dashboard: DashboardState? = null,
    config: JsonElement? = null,
    statistics: AgentStatisticsState? = null,
    options: StateContentOptions = StateContentOptions(),
): VpnHideState {
    val rootSections = rootSnapshot.sections
    val currentBootId = rootSections["current_boot_id"].orEmpty()
    val kpmLoadStatus = parseKpmLoadStatus(rootSections["kpm_load_status"].orEmpty())
    val backends = detectNativeBackendStates(rootSections, currentBootId, kpmLoadStatus)
    val activeBackend = displayNativeBackend(backends)
    val ports = detectPortsModule(rootSections)
    val kmodLoadStatus =
        readKmodLoadStatus(
            currentBootId.trim(),
            rootSections["kmod_load_status"].orEmpty(),
            rootSections["kmod_load_dmesg"].orEmpty(),
        )
    val installedOptionalHooks = installedNativeOptionalHooks(activeBackend.id, rootSections, currentBootId)

    val report =
        if (gate != null && checkResults != null) {
            buildDiagnosticReport(
                gate = gate,
                results = checkResults,
                backend = activeBackend,
                lsposedActive = lsposedHooksActiveThisBoot(rootSections["lsposed_state"].orEmpty(), currentBootId),
                complete = true,
                installedOptionalHooks = installedOptionalHooks,
            )
        } else {
            null
        }

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
        gate = report?.gate,
        nativeVerdict = report?.nativeVerdict,
        javaVerdict = report?.javaVerdict,
        report = report,
        backends = backends,
        activeBackend = activeBackend,
        ports = ports,
        kmodLoadStatus = kmodLoadStatus,
        dashboard = dashboard,
        // Explicit config (the bridge lean call) wins; otherwise derive it from the
        // raw canonical-config section so the file export carries it structured too.
        config = config ?: parseCanonicalConfigSection(rootSections["canonical_config"]),
        statistics = statistics,
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
        debugCapture = debugCapture,
        captureOptions = options,
        errors = errors,
    )
}
