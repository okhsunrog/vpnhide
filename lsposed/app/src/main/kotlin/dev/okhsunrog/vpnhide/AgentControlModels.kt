package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable

/** Result returned by agent bridge calls that mutate VPN Hide state. */
@Serializable
data class AgentMutationResult(
    /** True when the requested operation completed successfully. */
    val ok: Boolean,
    /** Human-readable status or failure detail. */
    val message: String,
    /** True when persistent app/root state changed. */
    val changed: Boolean = false,
    /** True when a selected app force-stop/reopen is needed for all effects to apply. */
    val targetRestartRecommended: Boolean = false,
)

/** Dashboard state summarized for machine consumers. */
@Serializable
data class AgentDashboardState(
    /** One of Protected, Attention, Unprotected, or VpnOff. */
    val heroStatus: String,
    /** Number of active logical layers: Java, Native, Ports. */
    val activeModuleCount: Int,
    /** Total logical layers shown by the dashboard. */
    val totalModuleCount: Int,
    /** Number of error messages shown by the dashboard. */
    val errorCount: Int,
    /** Number of warning messages shown by the dashboard. */
    val warningCount: Int,
    /** Number of neutral info messages shown by the dashboard. */
    val infoCount: Int,
    /** Native backend selected by the app priority logic, or null when none is active. */
    val activeNativeBackend: String?,
    /** Per-module state cards shown on the dashboard. */
    val modules: List<AgentModuleState>,
    /** Protection check summary shown on the dashboard. */
    val protection: AgentProtectionSummary,
    /** Dashboard messages in display priority order. */
    val messages: List<AgentDashboardMessage>,
)

/** One dashboard module/backend card. */
@Serializable
data class AgentModuleState(
    /** Stable card id. */
    val id: String,
    /** Backend kind: java, native, or ports. */
    val layer: String,
    /** Installed backend label. */
    val backend: String,
    /** High-level state such as active, installed_inactive, not_installed. */
    val state: String,
    /** Installed/running module version when known. */
    val version: String? = null,
    /** Number of configured targets for this backend. */
    val targetCount: Int = 0,
    /** Additional broken/degraded reason when known. */
    val reason: String? = null,
)

/** Dashboard protection summary. */
@Serializable
data class AgentProtectionSummary(
    /** none, needs_restart, checked, or vpn_off. */
    val state: String,
    /** Native-level summary: ok, fail, no_module, or null. */
    val native: String? = null,
    /** Java-level summary: ok, fail, hooks_inactive, or null. */
    val java: String? = null,
    /** Native checks passed when available. */
    val nativePassed: Int? = null,
    /** Native checks failed when available. */
    val nativeFailed: Int? = null,
    /** Java checks failed when available. */
    val javaFailed: Int? = null,
)

/** One dashboard message. */
@Serializable
data class AgentDashboardMessage(
    /** error, warning, or info. */
    val severity: String,
    /** User-facing message text. */
    val text: String,
)

/** Full diagnostics result. */
@Serializable
data class AgentDiagnosticsReport(
    /** ready or vpn_off. */
    val state: String,
    /** Combined pass score, excluding informational checks. */
    val score: AgentCheckScore,
    /** Native-level checks in UI order. */
    val nativeChecks: List<AgentCheckResult>,
    /** Java API-level checks in UI order. */
    val javaChecks: List<AgentCheckResult>,
)

/** Debug ZIP export metadata for agent-controlled diagnostics capture. */
@Serializable
data class AgentDebugZipExport(
    /** Absolute path inside app-private cache. Pull with run-as on debug builds. */
    val path: String,
    /** ZIP file size in bytes. */
    val sizeBytes: Long,
    /** File entries contained in the ZIP. */
    val entries: List<String>,
)

/** Pass/total diagnostics score. */
@Serializable
data class AgentCheckScore(
    /** Passed checks. */
    val passed: Int,
    /** Checks that produced a pass/fail result. */
    val total: Int,
)

/** One diagnostics probe result. */
@Serializable
data class AgentCheckResult(
    /** Probe name. */
    val name: String,
    /** pass, fail, or info. */
    val status: String,
    /** Probe detail text. */
    val detail: String,
    /** Who-hid-it outcome from the root differential (native checks): leak,
     * hidden_backend, hidden_selinux, nothing_to_leak, not_measured_*. Null for
     * checks without a root-differential outcome (Java / nativeExtra). */
    val outcome: String? = null,
)

/** Statistics tab state. */
@Serializable
data class AgentStatisticsState(
    /** True when at least one backend has status or counters. */
    val hasAnyData: Boolean,
    /** Active backend cards shown by the Statistics tab. */
    val activeBackendCount: Int,
    /** Number of counter rows. */
    val totalRows: Int,
    /** Total event count as decimal text to avoid unsigned ABI ambiguity. */
    val totalCount: String,
    /** Number of apps with probe activity after excluding VPN Hide's self-check noise. */
    val appCount: Int,
    /** Number of distinct detection methods currently seen. */
    val methodCount: Int,
    /** Backend statistics in UI order. */
    val backends: List<AgentBackendStatistics>,
    /** Per-app probe rollups shown by the Statistics tab. */
    val apps: List<AgentAppProbeStats>,
)

/** One Statistics backend card. */
@Serializable
data class AgentBackendStatistics(
    /** Backend id from the generated hook registry. */
    val backend: String,
    /** ok, partial, error, no_data, or unavailable. */
    val status: String,
    /** Installed hook count reported by backend status. */
    val hookedCount: Int,
    /** Total events for this backend as decimal text. */
    val totalCount: String,
    /** Reason when counters are unavailable. */
    val unavailableReason: String? = null,
    /** Sparse counter rows. */
    val rows: List<AgentStatisticRow>,
)

/** One sparse counter row. */
@Serializable
data class AgentStatisticRow(
    /** Runtime UID. */
    val uid: Long,
    /** Package names currently mapped to the UID. */
    val packageNames: List<String>,
    /** Hook id. */
    val hookId: Long,
    /** Hook registry name when known. */
    val hookName: String?,
    /** Cumulative event count as decimal text. */
    val count: String,
)

/** Per-app Statistics rollup with method taxonomy and exact hook breakdown. */
@Serializable
data class AgentAppProbeStats(
    /** Runtime UID. */
    val uid: Long,
    /** Package names currently mapped to the UID. */
    val packageNames: List<String>,
    /** Total event count as decimal text. */
    val totalCount: String,
    /** Detection surfaces seen for this app: java, native, package. */
    val surfaces: List<String>,
    /** User-facing method buckets in descending count order. */
    val methods: List<AgentDetectionMethodStats>,
    /** Exact backend hook counters in descending count order. */
    val hooks: List<AgentDetectionHookStats>,
)

/** One user-facing detection method bucket for an app. */
@Serializable
data class AgentDetectionMethodStats(
    /** Stable method id, for example Routes or NetworkCapabilities. */
    val method: String,
    /** Surface id: java, native, or package. */
    val surface: String,
    /** Event count as decimal text. */
    val count: String,
)

/** One exact hook counter behind an app's detection methods. */
@Serializable
data class AgentDetectionHookStats(
    /** Stable hook id from docs/protocol.md. */
    val hookId: Long,
    /** Hook registry name. */
    val hookName: String,
    /** Technical note from the hook registry. */
    val hookNote: String,
    /** Stable detection method id this hook folds into. */
    val method: String,
    /** Surface id: java, native, or package. */
    val surface: String,
    /** Event count as decimal text. */
    val count: String,
)

/** One cumulative counter captured at the start of a Statistics capture session. */
@Serializable
data class AgentStatisticCounterSnapshot(
    /** Runtime UID. */
    val uid: Long,
    /** Stable hook id from docs/protocol.md. */
    val hookId: Long,
    /** Cumulative count at baseline time. */
    val count: Long,
)

/** Baseline returned by getStatisticsCaptureBaseline and passed into getStatisticsCaptureDiff. */
@Serializable
data class AgentStatisticsCaptureBaseline(
    /** Cumulative counters keyed by uid and hook id. */
    val counters: List<AgentStatisticCounterSnapshot>,
)

/** Statistics capture-session diff, equivalent to the app's Start capture flow. */
@Serializable
data class AgentStatisticsCaptureDiff(
    /** True when a backend counter dropped below baseline and the caller should re-baseline. */
    val backendReset: Boolean,
    /** Per-app probe rollups that happened since the baseline. */
    val apps: List<AgentAppProbeStats>,
)

/** Apps tab state and canonical config summary. */
@Serializable
data class AgentProtectionState(
    /** Canonical JSON exactly as export/import uses it. */
    val canonicalConfigJson: String,
    /** Active native backend selected by app priority logic, or null. */
    val activeNativeBackend: String?,
    /** Native hook family currently displayed by the UI. */
    val nativeHookFamily: String,
    /** Configured apps sorted by package name. */
    val configuredApps: List<AgentConfiguredApp>,
    /** Canonical app-hiding settings. */
    val settings: AgentCanonicalSettings,
)

/** One configured app from canonical config. */
@Serializable
data class AgentConfiguredApp(
    /** Package name. */
    val packageName: String,
    /** Java/LSPosed role enabled. */
    val java: Boolean,
    /** Null means all Java hooks, empty means disabled. */
    val javaHooks: List<String>?,
    /** Native role enabled. */
    val native: Boolean,
    /** Backend-specific native hook overrides. */
    val nativeHooks: AgentNativeHookOverrides,
    /** App-hiding observer role enabled. */
    val appHiding: Boolean,
    /** Ports observer role enabled. */
    val ports: Boolean,
    /** Ports policy, or null for all localhost ports. */
    val portPolicy: AgentPortPolicy?,
    /** Package is hidden from app-hiding observers. */
    val hidden: Boolean,
)

/** Backend-specific native hook overrides from canonical config. */
@Serializable
data class AgentNativeHookOverrides(
    /** Null means all kernel hooks. */
    val kernel: List<String>?,
    /** Null means all Zygisk hooks. */
    val zygisk: List<String>?,
)

/** Canonical app-hiding settings. */
@Serializable
data class AgentCanonicalSettings(
    /** Whether APatch superkey persistence is enabled. */
    val rememberSuperkey: Boolean,
    /** Auto-hide packages declaring VpnService. */
    val autoHideVpnServices: Boolean,
    /** Auto-hide user apps whose label contains VPN. */
    val autoHideVpnName: Boolean,
    /** Packages currently hidden by enabled auto-hide heuristics. */
    val autoHiddenPackages: List<String>,
)

/** Ports policy shown by the Ports settings dialog. */
@Serializable
data class AgentPortPolicy(
    /** all, preset, or custom. */
    val mode: String,
    /** Preset id when mode is preset. */
    val preset: String?,
    /** Materialized port rules. */
    val rules: List<AgentPortRule>,
)

/** One TCP/UDP localhost port rule. */
@Serializable
data class AgentPortRule(
    /** both, tcp, or udp. */
    val protocol: String,
    /** Inclusive start port. */
    val start: Int,
    /** Inclusive end port. */
    val end: Int,
)

/** Installed app summary used by the Apps picker. */
@Serializable
data class AgentInstalledApp(
    /** Package name. */
    val packageName: String,
    /** Display label, with profile suffix when needed. */
    val label: String,
    /** True for system packages. */
    val system: Boolean,
    /** Android user/profile ids where this package is installed. */
    val userIds: List<String>,
    /** True when the app declares a VpnService. */
    val declaresVpnService: Boolean,
    /** True when the non-system label contains VPN. */
    val nameContainsVpn: Boolean,
    /** Existing configured roles. */
    val roles: List<String>,
)
