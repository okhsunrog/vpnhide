package dev.okhsunrog.vpnhide.migration

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CanonicalMutation
import dev.okhsunrog.vpnhide.DashboardState
import dev.okhsunrog.vpnhide.LEGACY_HOOK_STATUS_FILE
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.toCanonical
import dev.okhsunrog.vpnhide.toCore
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
    val invalidLines: Int = 0,
    val sourceReadComplete: Boolean = true,
) {
    val canCleanupSources: Boolean get() = sourceReadComplete && unresolvedObserverUids == 0 && invalidLines == 0

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
): Boolean = uniffi.vpnhide_core_ffi.legacyHasUserApps(config.toCore(), selfPkg)

internal fun parseLegacyConfigCandidate(
    sections: Map<String, String>,
    uidToPkg: Map<Int, String>,
): LegacyConfigCandidate? =
    parseLegacyConfigCandidate(sections, uidToPkg.entries.filter { it.key >= 0 }.groupBy({ it.value }, { it.key }), true)

internal fun parseLegacyConfigCandidate(
    sections: Map<String, String>,
    packageUids: Map<String, List<Int>>,
    inventoryComplete: Boolean,
): LegacyConfigCandidate? {
    val packages = packageUids.mapValues { (_, uids) -> uids.filter { it >= 0 }.map(Int::toUInt) }
    val candidate =
        uniffi.vpnhide_core_ffi.previewLegacyConfig(
            sections.filter { (key, value) ->
                key in LEGACY_CONFIG_SECTIONS &&
                    value.toByteArray().size <= 64 * 1024
            },
            packages,
            inventoryComplete,
        )
    if (candidate.roles.isEmpty()) return null
    return LegacyConfigCandidate(
        roles = candidate.roles.associate { it.`package` to LegacyRoles(it.java, it.native, it.appHiding, it.ports, it.hidden) },
        unresolvedObserverUids = candidate.unresolvedObserverUids.toInt(),
        invalidLines = candidate.invalidLines.toInt(),
        sourceReadComplete = candidate.sourceReadComplete,
    )
}

internal fun applyLegacyImport(
    base: CanonicalConfig,
    candidate: LegacyConfigCandidate,
    mode: LegacyImportMode,
    selfPkg: String,
): CanonicalConfig =
    uniffi.vpnhide_core_ffi
        .applyLegacyConfigPreview(
            base.toCore(),
            candidate.roles.map { (pkg, a) ->
                uniffi.vpnhide_core_ffi.CoreLegacyAppRoles(pkg, a.java, a.native, a.appHiding, a.ports, a.hidden)
            },
            mode.toCore(),
            selfPkg,
        ).toCanonical()

private fun LegacyImportMode.toCore(): uniffi.vpnhide_core_ffi.CoreLegacyImportMode =
    when (this) {
        LegacyImportMode.Merge -> uniffi.vpnhide_core_ffi.CoreLegacyImportMode.MERGE
        LegacyImportMode.Replace -> uniffi.vpnhide_core_ffi.CoreLegacyImportMode.REPLACE
    }

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
    suspend fun run(action: LegacyImportAction): LegacyImportOutcome {
        val snapshot =
            runCatching { RootSnapshotCache.getOrLoad() }
                .getOrElse { return LegacyImportOutcome.Failed(it.message ?: "root snapshot failed") }
        val targets = parseTargetsSnapshot(snapshot)
        val candidate =
            parseLegacyConfigCandidate(snapshot.sections, targets.packageUids, targets.nativeInventoryComplete)
                ?: return LegacyImportOutcome.NothingToImport
        val mode = action.mode ?: return discard()
        val result =
            CanonicalConfigRepository.commit(
                CanonicalMutation(
                    emptyList(),
                    protectAllDrafts = true,
                    cleanup = true,
                    coupledEffects = listOf(CoreRootEffect.MigrateLegacy(mode.toCore())),
                ),
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
     * Delete only. No canonical write and no backend apply: the config the
     * backends are already using does not change, just the dead files go away.
     * The caches still need a reload so the banner and the Settings entry
     * disappear without a manual refresh.
     */
    private suspend fun discard(): LegacyImportOutcome {
        val result =
            CanonicalConfigRepository.commit(
                CanonicalMutation(
                    emptyList(),
                    cleanup = true,
                    coupledEffects = listOf(CoreRootEffect.DeleteLegacyConfig),
                ),
            )
        val exit = result.exitCode
        val output = result.output
        if (exit != 0) {
            VpnHideLog.w(LogTags.APP, "legacy discard failed (exit=$exit): ${output.trim()}")
            return LegacyImportOutcome.Failed("exit=$exit")
        }
        VpnHideLog.i(LogTags.APP, "legacy config files deleted without importing")
        return LegacyImportOutcome.Discarded
    }
}
