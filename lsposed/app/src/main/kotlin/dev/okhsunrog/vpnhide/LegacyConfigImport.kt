package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.autoHiddenPackagesNeedReconcile
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import kotlinx.serialization.Serializable

// Import of the pre-1.0 on-disk configuration.
//
// Up to 0.7.1 the user's choices lived in flat text files, one per component.
// 1.0.0 moved everything into the canonical JSON and folded the old files in on
// first launch; 1.2.0 dropped that fold, so anyone jumping 0.7.x -> 1.2.x came
// up with an empty config and their setup looked reset. The old files survive on
// disk — 1.2.0 stopped deleting them as well — so the choices are recoverable.
//
// Two entry points:
//  - startup, silent: the canonical JSON holds no user-configured app, so the
//    legacy roles cannot conflict with anything (1.0.0's behaviour, restored).
//  - Dashboard banner / Settings: the canonical JSON already carries roles, so
//    the user picks merge / replace / skip.
//
// Either way the legacy files are deleted in the same root transaction as the
// config write. "Files gone" IS the imported marker, so no extra state is
// needed; a skip keeps them (and the Settings entry) available.

/** Package lists, one name per line (v0.7.x and earlier). */
internal const val LEGACY_KMOD_TARGETS = "/data/adb/vpnhide_kmod/targets.txt"
internal const val LEGACY_KPM_TARGETS = "/data/adb/vpnhide_kpm/targets.txt"
internal const val LEGACY_ZYGISK_TARGETS = "/data/adb/vpnhide_zygisk/targets.txt"
internal const val LEGACY_LSPOSED_TARGETS = "/data/adb/vpnhide_lsposed/targets.txt"
internal const val LEGACY_PORTS_OBSERVERS = "/data/adb/vpnhide_ports/observers.txt"
internal const val LEGACY_HIDDEN_PKGS = "/data/system/vpnhide_hidden_pkgs.txt"

// App-hiding observers were only ever persisted as resolved UIDs, so importing
// them needs the current package inventory to map back to names.
internal const val LEGACY_OBSERVER_UIDS = "/data/system/vpnhide_observer_uids.txt"

// Derived by the old service.sh from the LSPosed package list — never a user
// choice, so it is deleted with the rest but not read back.
internal const val LEGACY_JAVA_UIDS = "/data/system/vpnhide_uids.txt"

/**
 * Directories that existed only to hold the legacy files. `vpnhide_kmod`,
 * `vpnhide_kpm` and `vpnhide_ports` are deliberately absent: those are live
 * today (load_status / load_dmesg / ctl.lock) and must survive the import.
 */
internal val LEGACY_ORPHAN_DIRS = listOf("/data/adb/vpnhide_zygisk", "/data/adb/vpnhide_lsposed")

internal val LEGACY_FILES =
    listOf(
        LEGACY_KMOD_TARGETS,
        LEGACY_KPM_TARGETS,
        LEGACY_ZYGISK_TARGETS,
        LEGACY_LSPOSED_TARGETS,
        LEGACY_PORTS_OBSERVERS,
        LEGACY_HIDDEN_PKGS,
        LEGACY_OBSERVER_UIDS,
        LEGACY_JAVA_UIDS,
        LEGACY_HOOK_STATUS_FILE,
    )

/** Root-snapshot section name carrying each legacy file's contents. */
internal val LEGACY_CONFIG_SECTIONS =
    mapOf(
        "legacy_kmod_targets" to LEGACY_KMOD_TARGETS,
        "legacy_kpm_targets" to LEGACY_KPM_TARGETS,
        "legacy_zygisk_targets" to LEGACY_ZYGISK_TARGETS,
        "legacy_lsposed_targets" to LEGACY_LSPOSED_TARGETS,
        "legacy_ports_observers" to LEGACY_PORTS_OBSERVERS,
        "legacy_hidden_pkgs" to LEGACY_HIDDEN_PKGS,
        "legacy_observer_uids" to LEGACY_OBSERVER_UIDS,
    )

/** How an import folds into a canonical config that already has app roles. */
internal enum class LegacyImportMode {
    /** Union of roles per package. Nothing already configured is lost. */
    Merge,

    /** The legacy lists become the app roles; `settings` and self stay. */
    Replace,
}

/** Roles a single package held across the legacy files. */
internal data class LegacyRoles(
    val java: Boolean = false,
    val native: Boolean = false,
    val appHiding: Boolean = false,
    val ports: Boolean = false,
    val hidden: Boolean = false,
)

/**
 * What a pre-1.0 install left on disk, already resolved to package names.
 * Never empty: [parseLegacyConfigCandidate] returns null when there is nothing
 * to offer.
 */
internal data class LegacyConfigCandidate(
    val roles: Map<String, LegacyRoles>,
    /** Observer UIDs with no installed package today (app uninstalled since). */
    val unresolvedObserverUids: Int = 0,
) {
    fun count(predicate: (LegacyRoles) -> Boolean): Int = roles.values.count(predicate)
}

/**
 * True when the config carries a hiding choice the user made. Packages that only
 * carry `hidden` are excluded on purpose: those are the VPN apps auto-hide marks
 * by itself, so a config holding nothing else is still "untouched" and can take
 * a silent import.
 */
internal fun hasUserConfiguredApps(
    config: CanonicalConfig,
    selfPkg: String,
): Boolean =
    config.apps.any { (pkg, app) ->
        pkg != selfPkg && (app.java || app.native.enabled || app.appHiding || app.ports)
    }

internal fun parseLegacyConfigCandidate(
    sections: Map<String, String>,
    uidToPkg: Map<Int, String>,
): LegacyConfigCandidate? {
    fun packages(section: String): Set<String> =
        parseConfigLines(sections[section].orEmpty())
            .filter { it.isValidPackageName() }
            .toSet()

    val native =
        packages("legacy_kmod_targets") +
            packages("legacy_kpm_targets") +
            packages("legacy_zygisk_targets")
    val java = packages("legacy_lsposed_targets")
    val ports = packages("legacy_ports_observers")
    val hidden = packages("legacy_hidden_pkgs")
    val observerUids =
        parseConfigLines(sections["legacy_observer_uids"].orEmpty())
            .mapNotNull { it.toIntOrNull() }
            .toSet()
    val appHiding = observerUids.mapNotNull { resolveLegacyUid(it, uidToPkg) }.toSet()

    val packages = (native + java + ports + hidden + appHiding).sorted()
    if (packages.isEmpty()) return null
    return LegacyConfigCandidate(
        roles =
            packages.associateWith { pkg ->
                LegacyRoles(
                    java = pkg in java,
                    native = pkg in native,
                    appHiding = pkg in appHiding,
                    ports = pkg in ports,
                    hidden = pkg in hidden,
                )
            },
        unresolvedObserverUids = observerUids.count { resolveLegacyUid(it, uidToPkg) == null },
    )
}

/**
 * Map a persisted UID back to a package. Falls back to the app id (UID modulo
 * the per-user offset) so an observer picked in a work profile still resolves
 * after the profile was removed or renumbered.
 */
private fun resolveLegacyUid(
    uid: Int,
    uidToPkg: Map<Int, String>,
): String? {
    uidToPkg[uid]?.let { return it }
    val appId = uid % PER_USER_RANGE
    return uidToPkg.entries.firstOrNull { (known, _) -> known % PER_USER_RANGE == appId }?.value
}

private const val PER_USER_RANGE = 100_000

// Guards against feeding a truncated/corrupt file's junk into the config. Same
// shape Android itself accepts for a package name.
private val PACKAGE_NAME_REGEX = Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z0-9_]+)+")

private fun String.isValidPackageName(): Boolean = PACKAGE_NAME_REGEX.matches(this)

/**
 * Fold [candidate] into [base] and return the config to write. The self package
 * always keeps its full roles, whichever mode ran.
 */
internal fun applyLegacyImport(
    base: CanonicalConfig,
    candidate: LegacyConfigCandidate,
    mode: LegacyImportMode,
    selfPkg: String,
): CanonicalConfig {
    val kept =
        when (mode) {
            LegacyImportMode.Merge -> {
                base.apps
            }

            // Replace drops the user's app roles, but not the auto-hide marks:
            // those are derived from VpnService signals and tracked by
            // settings.autoHiddenPackages, and the reconcile only rewrites when
            // that SET changes (autoHiddenPackagesNeedReconcile). Dropping the
            // per-app `hidden` flag while the set still lists the package would
            // leave the VPN app visible to observers with nothing to fix it.
            LegacyImportMode.Replace -> {
                base.apps
                    .filterKeys { it in base.settings.autoHiddenPackages }
                    .filterValues { it.hidden }
                    .mapValues { (_, app) -> CanonicalApp(hidden = app.hidden) }
            }
        }
    val merged = kept.toMutableMap()
    candidate.roles.forEach { (pkg, roles) ->
        merged[pkg] = (kept[pkg] ?: CanonicalApp()).withLegacyRoles(roles)
    }
    val apps = merged.filterValues { it.hasAnyRole }.toSortedMap()
    return canonicalConfigWithSelfTarget(base.copy(apps = apps), selfPkg)
}

/**
 * Union of the current roles and the legacy ones. Legacy files have no
 * hook-level granularity, so a role that is already on keeps its current
 * per-hook selection and a role the import switches on gets every hook.
 */
private fun CanonicalApp.withLegacyRoles(roles: LegacyRoles): CanonicalApp =
    copy(
        java = java || roles.java,
        javaHooks = javaHooks.takeIf { java },
        native =
            if (native.enabled) {
                native
            } else if (roles.native) {
                NativeRole.All
            } else {
                NativeRole.Disabled
            },
        appHiding = appHiding || roles.appHiding,
        ports = ports || roles.ports,
        portPolicy = portPolicy.takeIf { ports },
        hidden = hidden || roles.hidden,
    )

/**
 * What the Dashboard card and its dialog render: how much a pending import would
 * bring in. Part of [DashboardState], so a debug bundle taken before the user
 * decides still shows what was found on disk.
 */
@Serializable
internal data class LegacyImportPrompt(
    val packages: Int,
    val java: Int,
    val native: Int,
    val appHiding: Int,
    val ports: Int,
    val hidden: Int,
    val unresolvedObserverUids: Int,
)

internal fun LegacyConfigCandidate.toPrompt(): LegacyImportPrompt =
    LegacyImportPrompt(
        packages = roles.size,
        java = count { it.java },
        native = count { it.native },
        appHiding = count { it.appHiding },
        ports = count { it.ports },
        hidden = count { it.hidden },
        unresolvedObserverUids = unresolvedObserverUids,
    )

/** What the user chose to do with the pre-1.0 files. */
internal enum class LegacyImportAction {
    Merge,
    Replace,

    /**
     * Delete the files without importing anything. The point of the whole flow
     * for a user who has already reconfigured the app by hand and just wants the
     * leftovers off the device.
     */
    Discard,
    ;

    val mode: LegacyImportMode?
        get() =
            when (this) {
                Merge -> LegacyImportMode.Merge
                Replace -> LegacyImportMode.Replace
                Discard -> null
            }
}

/** Outcome of a user-driven action, mapped to a message by the caller. */
internal sealed interface LegacyImportOutcome {
    data class Imported(
        val packages: Int,
    ) : LegacyImportOutcome

    data object Discarded : LegacyImportOutcome

    /** The files disappeared between rendering the card and confirming it. */
    data object NothingToImport : LegacyImportOutcome

    data class Failed(
        val detail: String,
    ) : LegacyImportOutcome
}

/**
 * Runs a user-confirmed action. Re-reads the current state rather than trusting
 * the card's snapshot: the config may have been edited on another screen (or by
 * the startup importer) since the card was built.
 */
internal object LegacyConfigImporter {
    suspend fun run(
        action: LegacyImportAction,
        selfPkg: String,
    ): LegacyImportOutcome {
        val snapshot =
            runCatching { RootSnapshotCache.getOrLoad() }
                .getOrElse { return LegacyImportOutcome.Failed(it.message ?: "root snapshot failed") }
        val targets = parseTargetsSnapshot(snapshot)
        val candidate =
            parseLegacyConfigCandidate(snapshot.sections, targets.uidToPkg)
                ?: return LegacyImportOutcome.NothingToImport
        val mode = action.mode ?: return discard()
        val updated =
            applyLegacyImport(targets.canonicalConfig ?: CanonicalConfig(), candidate, mode, selfPkg)
        val result =
            CanonicalConfigRepository.commit(
                updated,
                coupledCommands = listOf(buildLegacyConfigDeleteCommand()),
                activation = CanonicalActivation(native = true, ports = true),
            )
        if (!result.succeeded) {
            VpnHideLog.w(
                LogTags.APP,
                "legacy import failed (exit=${result.exitCode}): ${result.output.trim()}",
            )
            return LegacyImportOutcome.Failed("exit=${result.exitCode}")
        }
        VpnHideLog.i(LogTags.APP, "legacy import ($mode) applied for ${candidate.roles.size} packages")
        return LegacyImportOutcome.Imported(candidate.roles.size)
    }

    /**
     * Delete only. No canonical write and no activator run: the config the
     * backends are already using does not change, just the dead files go away.
     * The caches still need a reload so the banner and the Settings entry
     * disappear without a manual refresh.
     */
    private suspend fun discard(): LegacyImportOutcome {
        val (exit, output) = suExecAsync(buildLegacyConfigDeleteCommand())
        if (exit != 0) {
            VpnHideLog.w(LogTags.APP, "legacy discard failed (exit=$exit): ${output.trim()}")
            return LegacyImportOutcome.Failed("exit=$exit")
        }
        CanonicalConfigRepository.refreshDerivedCaches()
        VpnHideLog.i(LogTags.APP, "legacy config files deleted without importing")
        return LegacyImportOutcome.Discarded
    }
}

/**
 * Deletes every legacy file, then the two directories that held nothing else.
 * Runs as a coupled command inside the canonical write transaction (joined with
 * `&&`), so it must not report failure: `rm -f` already tolerates absent files,
 * and the rmdir is explicitly forgiven — the directories are gone on most
 * devices, and a non-empty one is left alone on purpose.
 */
internal fun buildLegacyConfigDeleteCommand(): String =
    listOf(
        "rm -f ${LEGACY_FILES.joinToString(" ")}",
        "{ rmdir ${LEGACY_ORPHAN_DIRS.joinToString(" ")} 2>/dev/null || true; }",
    ).joinToString(" && ")
