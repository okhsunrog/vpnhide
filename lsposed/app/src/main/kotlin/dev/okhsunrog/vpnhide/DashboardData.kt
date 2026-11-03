package dev.okhsunrog.vpnhide

import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.os.Build
import android.os.SystemClock
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.diagnostics.toDiagnosticInfo
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.hook.HookEntry
import dev.okhsunrog.vpnhide.migration.LegacyImportPrompt
import dev.okhsunrog.vpnhide.migration.parseLegacyConfigCandidate
import dev.okhsunrog.vpnhide.migration.toPrompt
import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.settings.AppLanguageContext
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import dev.okhsunrog.vpnhide.settings.resolveFilesystemHidingState
import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.flow.first
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import java.io.File
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

private const val TAG = LogTags.DASHBOARD

// ── Domain types — invalid states are unrepresentable ────────────────────

@Serializable
sealed interface ModuleState {
    @Serializable
    @SerialName("not_installed")
    data object NotInstalled : ModuleState

    @Serializable
    @SerialName("installed")
    data class Installed(
        val version: String?,
        val active: Boolean,
        // Only populated for kmod builds that carry the stamped `gkiVariant=` field
        // in module.prop (CI-built zips from v0.6.3+). Older builds report null.
        val gkiVariant: String? = null,
        // Non-null when the module is installed but the installation itself
        // is permanently broken (distinct from "active=false" which usually
        // just means a reboot is pending). UI colors the card red.
        val brokenReason: ModuleBrokenReason? = null,
        // True when an install is staged in modules_update/ awaiting the next
        // boot — the module just needs a reboot to take effect. UI colors the card orange, not red, and the dashboard
        // surfaces a reboot warning rather than a corruption error.
        val pendingReboot: Boolean = false,
        // False when the liveness probe for this module reads a runtime resource
        // the snapshot shell could not access (e.g. the 0600-root /proc/vpnhide_ctl
        // or an iptables query when the shell lacked root). Then `active=false` is
        // NOT trustworthy — the module may well be running — so the UI renders
        // "not verified", never a false "inactive". True when the probe was
        // authoritative. See [snapshotRuntimeCheckable].
        val runtimeCheckable: Boolean = true,
    ) : ModuleState
}

// Shared across every flashable module kind (kmod, KPM, ...) that stamps
// ModuleState.Installed.brokenReason — not just kmod, despite the name
// prefixes on the individual kmod-only cases below.
@Serializable
enum class ModuleBrokenReason {
    WrongVariant,
    UnsupportedKernel,
    MissingKprobes,
    UnknownVariantInactive,
    AmbiguousLoadFailed,
    SignatureEnforced,
    ArtifactMissing,
}

@Serializable
sealed interface LsposedState {
    @Serializable
    @SerialName("not_installed")
    data object NotInstalled : LsposedState

    @Serializable
    @SerialName("installed_inactive")
    data class InstalledInactive(
        val version: String?,
    ) : LsposedState

    @Serializable
    @SerialName("needs_reboot")
    data class NeedsReboot(
        val version: String?,
    ) : LsposedState

    @Serializable
    @SerialName("active")
    data class Active(
        val version: String?,
        val targetCount: Int,
    ) : LsposedState
}

@Serializable
internal sealed interface ProtectionCheck {
    @Serializable
    @SerialName("autonomous")
    data class Autonomous(
        val evidence: dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticInfo,
    ) : ProtectionCheck
}

internal enum class FlashableModuleKind { Kmod, Builtin, Kpm, Zygisk, Core }

internal data class PortsApplyProblem(
    val failureDetail: String?,
)

internal fun detectPortsApplyProblem(
    ports: ModuleState,
    targetCount: Int,
    lastApply: PortsBootStatus?,
    portsDisabled: Boolean,
): PortsApplyProblem? {
    val installed = ports as? ModuleState.Installed ?: return null
    if (installed.active || installed.brokenReason != null || targetCount == 0) return null
    // An inactive reading the snapshot could not verify (probe failed, shell not
    // root) is no evidence that the rules are missing: nothing to warn about.
    if (!installed.runtimeCheckable) return null
    // A module the user turned off via their manager (a `disable` marker) is
    // inactive by design — core does not start for the same reason. Don't
    // nag "iptables rules are not active" for a deliberately disabled module.
    if (portsDisabled) return null

    // Core publishes its last Ports apply this boot: a family that did not confirm
    // carries the reason the rules could be missing.
    val detail =
        lastApply?.takeIf { !it.applied }?.let { apply ->
            apply.detail.trim().ifBlank { "IPv4 ${apply.ipv4}, IPv6 ${apply.ipv6}" }
        }
    return PortsApplyProblem(failureDetail = detail)
}

internal data class ModuleMismatch(
    val kind: FlashableModuleKind,
    val moduleVersion: String,
    val appVersion: String,
)

// Pure: given a list of (state, kind) pairs and the app version, returns
// the subset whose base version disagrees with the app. Extracted so the
// three kmod / zygisk / ports callsites in loadDashboardState share one
// code path instead of three near-identical if-blocks.
internal fun detectModuleMismatches(
    modules: List<Pair<ModuleState, FlashableModuleKind>>,
    appVersion: String,
): List<ModuleMismatch> =
    modules.mapNotNull { (state, kind) ->
        val installed = state as? ModuleState.Installed ?: return@mapNotNull null
        val moduleVersion = installed.version ?: return@mapNotNull null
        if (versionsMismatch(moduleVersion, appVersion)) {
            ModuleMismatch(kind, moduleVersion, appVersion)
        } else {
            null
        }
    }

internal sealed interface LsposedFramework {
    data object NotInstalled : LsposedFramework

    data class Installed(
        val disabled: Boolean,
    ) : LsposedFramework
}

internal sealed interface LsposedConfig {
    data object ModuleNotConfigured : LsposedConfig

    data object Disabled : LsposedConfig

    data class Enabled(
        val entries: List<String>,
        val hasSystemFramework: Boolean,
        val extraEntries: List<String>,
    ) : LsposedConfig
}

@Serializable
internal enum class DashboardMessageSeverity { ERROR, WARNING, INFO }

// Optional call-to-action rendered on a message banner. ContactAuthor opens the
// shared community/feedback modal (GitHub / Telegram / 4PDA); OpenDiagnostics
// opens the detailed diagnostics screen.
@Serializable
internal enum class DashboardMessageAction { ContactAuthor, OpenDiagnostics }

@Serializable
internal data class DashboardMessage(
    val severity: DashboardMessageSeverity,
    val text: String,
    val action: DashboardMessageAction? = null,
    // When set, the banner renders a "download this zip" button that grabs this
    // named artifact from the latest release (wrong variant, outdated module, …).
    val downloadArtifact: String? = null,
    // When set (and there is no download/other action), the banner shows a
    // "Learn more" button that opens this help article — the contextual entry
    // from an issue to the guide section that explains and fixes it.
    val helpArticle: String? = null,
    val reason: DashboardIssueReason? = null,
    val presentationLanguage: String? = null,
)

@Serializable
internal data class DashboardState(
    val kmod: ModuleState,
    val kpm: ModuleState,
    val zygisk: ModuleState,
    val lsposed: LsposedState,
    val ports: ModuleState,
    val nativeTargetCount: Int,
    val portsTargetCount: Int,
    // The one native backend surfaced on the dashboard (kmod > KPM > Zygisk).
    val nativeBackend: DisplayNativeBackend,
    val protection: ProtectionCheck,
    val messages: List<DashboardMessage>,
    // A pre-1.0 config still on disk next to a config that already has roles —
    // the startup importer leaves that case to the user (LegacyConfigImport).
    val legacyImport: LegacyImportPrompt? = null,
    val core: CoreDashboardData = CoreDashboardData(),
    val nativeStatus: NativeStatusSnapshot? = null,
)

internal fun moduleActive(state: ModuleState): Boolean = (state as? ModuleState.Installed)?.active == true

/** uid the root-snapshot shell actually ran as, parsed from the `snapshot_shell_uid`
 * probe. Null when the probe is absent (older snapshot) — treated as trusted (0). */
internal fun snapshotShellUid(sections: Map<String, String>): Int? =
    sections["snapshot_shell_uid"]
        .orEmpty()
        .lineSequence()
        .firstOrNull { it.startsWith("uid=") }
        ?.substringAfter("uid=")
        ?.trim()
        ?.toIntOrNull()

/** Whether the snapshot shell had the privilege to trust its runtime-liveness
 * probes (0600 /proc/vpnhide_ctl, iptables). A shell that isn't uid 0 reads a
 * false negative from those, so a negative liveness result is "not verified",
 * not "inactive". A missing probe (older snapshot) defaults to trusted. */
internal fun snapshotRuntimeCheckable(sections: Map<String, String>): Boolean {
    val uid = snapshotShellUid(sections) ?: return true
    return uid == 0
}

internal data class ModulePropInfo(
    val installed: Boolean,
    val version: String?,
    val gkiVariant: String?,
)

// Older CI-built zips didn't stamp `gkiVariant=` but their injected updateJson
// URL already encodes the KMI: `.../update-kmod-<kmi>.json`. Recover the variant
// from there so wrong-variant detection works for existing installs.
private val UPDATE_JSON_KMI_REGEX = Regex("""update-kmod-([^/]+)\.json""")

internal fun parseModuleProp(raw: String): ModulePropInfo {
    if (raw.isBlank()) return ModulePropInfo(false, null, null)
    var version: String? = null
    var gkiVariant: String? = null
    var updateJsonKmi: String? = null
    for (line in raw.lines()) {
        when {
            line.startsWith("version=") -> {
                version = normalizeVersion(line.removePrefix("version="))
            }

            line.startsWith("gkiVariant=") -> {
                gkiVariant = line.removePrefix("gkiVariant=").trim().ifBlank { null }
            }

            line.startsWith("updateJson=") -> {
                updateJsonKmi = UPDATE_JSON_KMI_REGEX.find(line.removePrefix("updateJson="))?.groupValues?.get(1)
            }
        }
    }
    return ModulePropInfo(true, version, gkiVariant ?: updateJsonKmi)
}

// ── LSPosed state resolution (pure, unit-tested) ─────────────────────────

/**
 * Resolve the user-facing [LsposedState] from the framework presence, the
 * on-disk module config, and whether the hook heartbeat is fresh for this
 * boot. A current-boot heartbeat is the strongest signal (module is active);
 * otherwise the config/framework decide inactive/needs-reboot/not-configured.
 */
internal fun resolveLsposedState(
    hooksActiveThisBoot: Boolean,
    hookVersion: String?,
    lsposedTargetCount: Int,
    framework: LsposedFramework,
    config: LsposedConfig?,
): LsposedState {
    if (hooksActiveThisBoot) {
        return LsposedState.Active(hookVersion, lsposedTargetCount)
    }
    return when (config) {
        null -> {
            LsposedState.InstalledInactive(null)
        }

        LsposedConfig.ModuleNotConfigured -> {
            when (framework) {
                LsposedFramework.NotInstalled -> LsposedState.NotInstalled
                is LsposedFramework.Installed -> LsposedState.InstalledInactive(null)
            }
        }

        LsposedConfig.Disabled -> {
            LsposedState.InstalledInactive(null)
        }

        is LsposedConfig.Enabled -> {
            if (config.hasSystemFramework) {
                LsposedState.NeedsReboot(hookVersion)
            } else {
                LsposedState.InstalledInactive(null)
            }
        }
    }
}

internal fun detectPortsModule(sections: Map<String, String>): ModuleState {
    val prop = parseModuleProp(sections["core_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    // "1" = chains present and jumped to, "0" = a chain or jump is absent,
    // "error=<rc>" (or nothing) = the probe itself could not run: the xtables lock
    // was held, the shell lacked CAP_NET_ADMIN, the phase never ran. Only a "0"
    // from a root shell is evidence of an inactive module; the rest is unverified.
    val probe = sections["ports_chain"].orEmpty().trim()
    val active = probe == "1"
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        runtimeCheckable = active || (probe == "0" && snapshotRuntimeCheckable(sections)),
    )
}

// ── lsposed framework / config probes (need Android: PM, SQLite, root) ────

private fun detectLsposedFramework(sections: Map<String, String>): LsposedFramework {
    val out = sections["lsposed_framework"].orEmpty()
    val props = parseKeyValueLines(out)
    val installedValue = props["installed"]
    val malformed =
        props["probe_ok"] != "1" ||
            installedValue == null ||
            (installedValue == "1" && props["disabled"] == null)
    if (malformed) {
        VpnHideLog.w(TAG, "lsposed framework probe returned malformed output: $out")
        return LsposedFramework.NotInstalled
    }
    val framework =
        if (installedValue == "1") {
            LsposedFramework.Installed(disabled = props["disabled"] == "1")
        } else {
            LsposedFramework.NotInstalled
        }
    VpnHideLog.i(TAG, "lsposed framework: $framework (raw=$out)")
    return framework
}

// Core copies the LSPosed config DB (+ WAL/SHM) out of root-only storage into the
// app's export dir, owned by the app, so SQLiteDatabase can open it read-only.
// Returns the copied main db file, or null if core could not copy it.
private fun copyLsposedConfigDb(context: android.content.Context): File? {
    CoreRoot.attach(context)
    val copyStart = SystemClock.elapsedRealtime()
    val result = CoreRoot.collectOneBlocking(CoreProbe.LsposedModulesDb)
    StartupTrace.metric("dashboard_lsposed_db_copy", SystemClock.elapsedRealtime() - copyStart)
    val dbCopy = result.artifacts.firstOrNull { it.name == "modules_config.db" }
    if (!result.ok || dbCopy == null || !dbCopy.isFile) {
        VpnHideLog.w(TAG, "failed to copy LSPosed config db for inspection: exit=${result.exit} out=${result.text}")
        return null
    }
    VpnHideLog.i(TAG, "lsposed db copy: ${result.text.trim()}")
    return dbCopy
}

// Nesting depth comes from the chained SQLite `.use {}` resource scopes
// (db → modules cursor → scope cursor), not from branching logic.
@Suppress("NestedBlockDepth")
internal fun readLsposedConfig(
    context: android.content.Context,
    selfPkg: String,
): LsposedConfig? {
    val dbCopy = copyLsposedConfigDb(context) ?: return null
    val queryStart = SystemClock.elapsedRealtime()
    return try {
        SQLiteDatabase.openDatabase(dbCopy.absolutePath, null, SQLiteDatabase.OPEN_READONLY).use { db ->
            db
                .rawQuery("SELECT mid, enabled FROM modules WHERE module_pkg_name = ?", arrayOf(selfPkg))
                .use { moduleCursor ->
                    if (!moduleCursor.moveToFirst()) return LsposedConfig.ModuleNotConfigured
                    val mid = moduleCursor.getLong(0)
                    if (moduleCursor.getInt(1) == 0) return LsposedConfig.Disabled

                    val scopeEntries = mutableListOf<Pair<String, Int>>()
                    db
                        .rawQuery(
                            "SELECT app_pkg_name, user_id FROM scope WHERE mid = ? ORDER BY user_id, app_pkg_name",
                            arrayOf(mid.toString()),
                        ).use { scopeCursor ->
                            while (scopeCursor.moveToNext()) {
                                scopeEntries += scopeCursor.getString(0) to scopeCursor.getInt(1)
                            }
                        }

                    fun isSystemFramework(
                        pkg: String,
                        userId: Int,
                    ) = pkg == "system" && userId == 0
                    LsposedConfig.Enabled(
                        entries = scopeEntries.map { (pkg, userId) -> if (isSystemFramework(pkg, userId)) "system" else "$pkg/$userId" },
                        hasSystemFramework = scopeEntries.any { (pkg, userId) -> isSystemFramework(pkg, userId) },
                        extraEntries =
                            scopeEntries
                                .filterNot { (pkg, userId) -> isSystemFramework(pkg, userId) || pkg == selfPkg }
                                .map { (pkg, userId) -> "$pkg/$userId" },
                    )
                }
        }
    } catch (e: Exception) {
        VpnHideLog.w(TAG, "failed to inspect LSPosed config db: ${e.message}")
        null
    } finally {
        StartupTrace.metric("dashboard_lsposed_db_query", SystemClock.elapsedRealtime() - queryStart)
        dbCopy.delete()
        File(dbCopy.absolutePath + "-wal").delete()
        File(dbCopy.absolutePath + "-shm").delete()
    }
}

// Linear orchestrator: pure detectors above produce the module/lsposed state,
// then a flat list of independent guards builds the dashboard message banners.
// Kept as one top-to-bottom narrative — splitting the flat guard list behind a
// parameter bundle would add indirection without improving clarity.

/**
 * One flashable module's derived state: the integrity/load diagnosis, whether
 * it is merely staged for the next reboot, and the [ModuleState] with both
 * folded in.
 *
 * The four backends used to do this inline, in four ~18-line blocks that
 * differed only in kind and the backend-specific classifier — which is exactly
 * the shape that lets one of them quietly drift.
 *
 * Order matters: a staged install is not a corrupt one, so [modulePendingReboot]
 * suppresses the diagnosis entirely; integrity beats the backend classifier so a
 * missing payload is not reported as a load failure.
 */
private fun deriveModuleFact(
    kind: FlashableModuleKind,
    raw: ModuleState,
    sections: Map<String, String>,
    classifyBackendProblem: () -> ModuleProblem? = { null },
): ModuleFact {
    val pendingReboot = modulePendingReboot(kind, raw, sections)
    val problem =
        if (pendingReboot) {
            null
        } else {
            moduleIntegrityProblem(
                kind = kind,
                module = raw,
                sections = sections,
            )?.let { ModuleProblem.Integrity(it) }
                ?: classifyBackendProblem()
        }
    return ModuleFact(
        state = raw.withBrokenReason(problem?.reason).withPendingReboot(pendingReboot),
        problem = problem,
        pendingReboot = pendingReboot,
    )
}

/** Every module's state, as the cards show it and the banners read it. */
private fun deriveModuleFacts(
    sections: Map<String, String>,
    appVersion: String,
    nativeBoot: NativeBootStatus?,
    nativeStatus: NativeStatusSnapshot,
): ModuleFacts {
    val raw = nativeLegacyInventory(sections)

    fun fact(state: ModuleState) = ModuleFact(state, problem = null, pendingReboot = false)
    val ports = deriveModuleFact(FlashableModuleKind.Core, detectPortsModule(sections), sections)
    return ModuleFacts(
        kmod = fact(raw.kmod),
        builtin = fact(raw.builtin),
        kpm = fact(raw.kpm),
        zygisk = fact(raw.zygisk),
        ports = ports,
        backends = raw,
        nativeBackend = coreDisplayNativeBackend(raw, nativeStatus),
        currentBootId = sections["current_boot_id"].orEmpty(),
        nativeBoot = nativeBoot,
        builtinKernelPresent = nativeStatus.current && nativeStatus.model?.observed == NativeBackendId.Builtin,
        mismatches = detectModuleMismatches(listOf(ports.state to FlashableModuleKind.Core), appVersion),
    )
}

/** LSPosed's runtime and on-disk state, and the hooks' own install health. */
private fun deriveLsposedFacts(
    context: android.content.Context,
    sections: Map<String, String>,
    currentBootId: String,
    lsposedTargetCount: Int,
): LsposedFacts {
    val lsposedStateRaw = sections["lsposed_state"].orEmpty()
    val hookProps = parseLsposedStateMetadata(lsposedStateRaw)
    val hooksActiveThisBoot = lsposedHooksActiveThisBoot(lsposedStateRaw, currentBootId)
    val framework = detectLsposedFramework(sections)
    val config =
        if (hooksActiveThisBoot) {
            // A current-boot hook heartbeat is stronger evidence than the on-disk
            // LSPosed DB: the module is active, and config warnings are suppressed
            // for active hooks anyway.
            null
        } else {
            when (framework) {
                LsposedFramework.NotInstalled -> {
                    LsposedConfig.ModuleNotConfigured
                }

                is LsposedFramework.Installed -> {
                    if (framework.disabled) {
                        LsposedConfig.Disabled
                    } else {
                        readLsposedConfig(context, context.packageName)
                    }
                }
            }
        }
    StartupTrace.mark("dashboard_lsposed_config_done")
    val state =
        resolveLsposedState(
            hooksActiveThisBoot = hooksActiveThisBoot,
            hookVersion = hookProps["version"],
            lsposedTargetCount = lsposedTargetCount,
            framework = framework,
            config = config,
        )
    VpnHideLog.i(
        TAG,
        "lsposed: $state (hookBootId=${hookProps["boot_id"]} currentBootId=${currentBootId.trim()} " +
            "status=${ProtocolReader.parseStatus(lsposedStateRaw)} framework=$framework " +
            "hooksActive=$hooksActiveThisBoot config=$config)",
    )
    return LsposedFacts(
        state = state,
        framework = framework,
        config = config,
        // The install-time smoke check on the private NetworkCapabilities /
        // NetworkInfo / LinkProperties fields the hooks reflect on. Non-empty means
        // the running AOSP renamed or retyped one and the matching writeToParcel
        // hook was skipped — independent of Active/Inactive, since hooks can be live
        // with partial coverage.
        brokenFields = hookProps["broken_fields"]?.takeIf { it.isNotBlank() },
        installFailures = hookProps["install_failures"]?.takeIf { it.isNotBlank() },
        aospSdkLabel = hookProps["aosp_sdk"]?.takeIf { it.isNotBlank() } ?: "?",
    )
}

/**
 * The device state not owned by any one module.
 *
 * App-local settings are deliberately absent: they are not read from root and
 * these facts are cached until an observation invalidates them, so a preference
 * in here would only take effect after the next refresh. They reach the
 * Dashboard through [DeveloperFlagsCache] instead.
 */
private suspend fun deriveEnvironmentFacts(
    context: android.content.Context,
    sections: Map<String, String>,
    targetsSnapshot: TargetsSnapshot,
    ports: ModuleState,
    portsTargetCount: Int,
    nativeBoot: NativeBootStatus?,
    nativeStatus: NativeStatusSnapshot,
): EnvironmentFacts =
    EnvironmentFacts(
        selinuxPermissive = sections["getenforce"].orEmpty().trim().equals("Permissive", ignoreCase = true),
        // Each profile's picker only lists its own apps (getInstalledApplications is
        // per-user), so a Save from a profile that cannot see every target would
        // silently drop the rest.
        selfProfileCount =
            parsePackageUidMap(sections["pm_packages"].orEmpty())[context.packageName]
                ?.distinct()
                ?.size
                ?: 0,
        debugLoggingOn = targetsSnapshot.canonicalConfig?.debug == true,
        filesystemHiding = resolveFilesystemHidingState(nativeStatus),
        portsApply =
            detectPortsApplyProblem(
                ports,
                portsTargetCount,
                nativeBoot?.takeIf { it.isFreshFor(sections["current_boot_id"].orEmpty()) }?.ports,
                portsDisabled = sections["core_disabled"].orEmpty().trim() == "1",
            ),
        obsoleteModules = parseObsoleteModules(sections),
    )

/** Root snapshot `obsolete_modules`: one installed 1.x module id per line. */
internal fun parseObsoleteModules(sections: Map<String, String>): List<String> =
    sections[OBSOLETE_MODULES_SECTION]
        .orEmpty()
        .lines()
        .map { it.trim() }
        .filter { it.isNotEmpty() }

internal const val OBSOLETE_MODULES_SECTION = "obsolete_modules"

/**
 * Everything the Dashboard derives from one root snapshot alone: the module,
 * LSPosed, target and environment facts, the kernel recommendation, the optional
 * hooks this boot installed, and the legacy-import prompt. It costs root shells
 * and parsing, so it is cached ([DashboardCache]); the protection tiles and the
 * banners that depend on the diagnostic presentation are assembled on top of it
 * by [assembleDashboardState], a pure step that follows every presentation.
 */
internal data class DashboardRootFacts(
    val selfNeedsRestart: Boolean,
    val modules: ModuleFacts,
    val lsposed: LsposedFacts,
    val targets: TargetCounts,
    val environment: EnvironmentFacts,
    val appVersion: String,
    val legacyImport: LegacyImportPrompt?,
    val coreUpdate: CoreSessionUpdate = CoreSessionUpdate(),
)

/**
 * The screen's state object, assembled from the facts it was derived from.
 *
 * Separate from [loadDashboardState] because it is pure plumbing: no decision is
 * made here, every field is either copied out of [DashboardFacts] or is the one
 * value the facts deliberately do not carry ([messages], already worded).
 */
private fun DashboardFacts.toDashboardState(
    messages: List<DashboardMessage>,
    legacyImport: LegacyImportPrompt?,
): DashboardState =
    DashboardState(
        kmod = modules.kmod.state,
        kpm = modules.kpm.state,
        zygisk = modules.zygisk.state,
        lsposed = lsposed.state,
        ports = modules.ports.state,
        nativeTargetCount = targets.native,
        portsTargetCount = targets.ports,
        nativeBackend = modules.nativeBackend,
        protection = protection.check,
        messages = messages,
        legacyImport = legacyImport,
    )

/**
 * Everything the Dashboard shows, from one root snapshot and one diagnostic
 * presentation: the root facts, then the assembly on top of them. The bridge
 * uses this one-shot form; the screen's cache keeps the root facts and assembles
 * on every presentation instead.
 */
internal suspend fun loadDashboardState(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
    diagnostics: dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticSnapshot,
): DashboardState {
    val facts = deriveDashboardRootFacts(context, selfNeedsRestart, rootSnapshot)
    val developer = SettingsRepository(context.applicationContext).settings.first().toDeveloperFlags()
    return assembleDashboardState(context, facts, diagnostics, developer, diagnostics.sources.core)
}

/**
 * The root-derived half of the Dashboard. The banner logic itself is
 * deliberately not here — [dashboardIssues] decides and [toMessage] words it, so
 * the ~25 guards are reachable from a unit test that needs no `Context`.
 */
internal suspend fun deriveDashboardRootFacts(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
): DashboardRootFacts {
    VpnHideLog.i(TAG, "=== Loading dashboard state ===")
    StartupTrace.mark("dashboard_derive_start")
    val selfPkg = context.packageName
    val sections = rootSnapshot.sections
    val targetsSnapshot = parseTargetsSnapshot(rootSnapshot)

    fun countPackages(pkgs: Set<String>): Int = pkgs.count { it != selfPkg }
    val targets =
        TargetCounts(
            lsposed = countPackages(targetsSnapshot.lsposedTargets),
            native = countPackages(targetsSnapshot.nativeTargets),
            ports = countPackages(targetsSnapshot.portsObservers),
        )

    val appVersion = BuildConfig.VERSION_NAME
    val nativeStatus = rootSnapshot.core.nativeStatusSnapshot()
    val modules = deriveModuleFacts(sections, appVersion, rootSnapshot.nativeBoot, nativeStatus)
    StartupTrace.mark("dashboard_modules_done")
    StartupTrace.mark("dashboard_kernel_done")

    val lsposed = deriveLsposedFacts(context, sections, modules.currentBootId, targets.lsposed)
    StartupTrace.mark("dashboard_lsposed_done")

    val environment =
        deriveEnvironmentFacts(
            context = context,
            sections = sections,
            targetsSnapshot = targetsSnapshot,
            ports = modules.ports.state,
            portsTargetCount = targets.ports,
            nativeBoot = rootSnapshot.nativeBoot,
            nativeStatus = nativeStatus,
        )
    VpnHideLog.i(TAG, "selfNeedsRestart=$selfNeedsRestart")
    VpnHideLog.i(TAG, "=== Dashboard root facts loaded ===")
    return DashboardRootFacts(
        selfNeedsRestart = selfNeedsRestart,
        modules = modules,
        lsposed = lsposed,
        targets = targets,
        environment = environment,
        appVersion = appVersion,
        legacyImport =
            parseLegacyConfigCandidate(
                sections,
                targetsSnapshot.packageUids,
                targetsSnapshot.nativeInventoryComplete,
            )?.toPrompt(),
        coreUpdate = rootSnapshot.core,
    )
}

/**
 * The presentation-dependent half: the protection tiles, the banners and the
 * screen state, assembled from cached root facts and the presentation of the
 * moment. Pure apart from wording the messages, so it can follow every
 * presentation change and the tiles can never lag behind the hero.
 */
internal fun assembleDashboardState(
    context: android.content.Context,
    facts: DashboardRootFacts,
    diagnostics: dev.okhsunrog.vpnhide.diagnostics.AutonomousDiagnosticSnapshot,
    developer: DeveloperFlags,
    coreUpdate: CoreSessionUpdate = facts.coreUpdate,
): DashboardState =
    assembleDashboardState(
        context,
        facts,
        ProtectionFacts(
            check = ProtectionCheck.Autonomous(diagnostics.toDiagnosticInfo()),
        ),
        developer,
        coreUpdate,
    )

private fun assembleDashboardState(
    context: android.content.Context,
    facts: DashboardRootFacts,
    protection: ProtectionFacts,
    developer: DeveloperFlags,
    coreUpdate: CoreSessionUpdate = facts.coreUpdate,
): DashboardState {
    val localized = AppLanguageContext.localized(context)
    val dashboardFacts =
        DashboardFacts(
            facts.modules,
            facts.lsposed,
            facts.targets,
            facts.environment.copy(filesystemHiding = resolveFilesystemHidingState(coreUpdate.nativeStatusSnapshot())),
            protection,
            facts.appVersion,
            developer,
        )
    val messages =
        dashboardIssues(dashboardFacts).map { issue ->
            issue.toMessage(localized, localized.resources).copy(
                reason = issue.toReason(),
                presentationLanguage =
                    localized.resources.configuration.locales[0]
                        .toLanguageTag(),
            )
        }
    VpnHideLog.i(TAG, "protection=${protection.check} messages=$messages")
    return dashboardFacts.toDashboardState(messages = messages, legacyImport = facts.legacyImport).copy(
        core = coreDashboardData(coreUpdate, (facts.modules.ports.state as? ModuleState.Installed)?.version),
        nativeStatus = coreUpdate.nativeStatusSnapshot(),
    )
}
