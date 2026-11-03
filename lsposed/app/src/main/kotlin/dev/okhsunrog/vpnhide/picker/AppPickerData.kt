package dev.okhsunrog.vpnhide.picker

import dev.okhsunrog.vpnhide.CanonicalApp
import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.NativeHookOverrides
import dev.okhsunrog.vpnhide.NativeRole
import dev.okhsunrog.vpnhide.PortPolicy
import dev.okhsunrog.vpnhide.buildCanonicalConfigFromTargetsSnapshot
import dev.okhsunrog.vpnhide.toCanonical
import dev.okhsunrog.vpnhide.toCore
import uniffi.vpnhide_core_ffi.CoreAutoHideSignal
import uniffi.vpnhide_core_ffi.previewAutoHidden
import uniffi.vpnhide_core_ffi.previewHiddenPackages
import uniffi.vpnhide_core_ffi.previewManualHidden
import uniffi.vpnhide_core_ffi.previewManualHiddenUpdate
import uniffi.vpnhide_core_ffi.previewPickerSave
// UI records delegate config domain projections to shared Rust through local FFI.

/**
 * The hidden-package set to persist on Save. This preserves manual hidden
 * packages and auto-detected VPN apps, and always includes [selfPkg] (managed
 * invisibly, never shown in the picker).
 *
 * Hidden and app-hiding observer roles can coexist. The package-visibility hook
 * avoids self-crashes by never hiding a package from callers with the same
 * appId; other observers still cannot see it.
 */
internal fun resolveHiddenPackages(
    existing: Set<String>,
    selfPkg: String,
): List<String> = previewHiddenPackages(existing.toList(), selfPkg)

internal data class AppAutoHideSignal(
    val packageName: String,
    val declaresVpnService: Boolean = false,
    val nameContainsVpn: Boolean = false,
)

private fun Collection<AppAutoHideSignal>.toCoreSignals(): List<CoreAutoHideSignal> =
    map { CoreAutoHideSignal(it.packageName, it.declaresVpnService, it.nameContainsVpn) }

internal data class AppRoleSelection(
    val packageName: String,
    val uids: List<Int> = emptyList(),
    val java: Boolean = false,
    val javaHooks: List<String>? = null,
    val native: Boolean = false,
    val nativeOverrides: NativeHookOverrides = NativeHookOverrides(),
    val appHiding: Boolean = false,
    val ports: Boolean = false,
    val portPolicy: PortPolicy? = null,
)

private const val ANDROID_UIDS_PER_USER = 100_000
private const val FIRST_APPLICATION_UID = 10_000

/** Distinct packages sharing an appId; copies of one package in profiles do not count. */
internal fun packagesSharingAppIds(packageUids: Map<String, List<Int>>): Set<String> =
    packageUids.entries
        .flatMap { (pkg, uids) ->
            uids
                .filter { it >= FIRST_APPLICATION_UID }
                .map { it % ANDROID_UIDS_PER_USER }
                .distinct()
                .map { it to pkg }
        }.groupBy({ it.first }, { it.second })
        .values
        .filter { it.size > 1 }
        .flatten()
        .toSet()

internal fun resolveNativeHookSelection(
    hookNames: List<String>,
    selectedHookNames: Set<String>,
): List<String>? = resolveHookSelection(hookNames, selectedHookNames)

internal fun resolveHookSelection(
    hookNames: List<String>,
    selectedHookNames: Set<String>,
): List<String>? {
    val ordered = hookNames.filter { it in selectedHookNames }
    return when {
        ordered.isEmpty() -> emptyList()
        ordered.size == hookNames.size -> null
        else -> ordered
    }
}

internal fun buildCanonicalConfigForAppPickerSave(
    debug: Boolean,
    selfPkg: String,
    selections: Collection<AppRoleSelection>,
    snapshot: TargetsSnapshot?,
    autoHideSignals: Collection<AppAutoHideSignal> = emptyList(),
    partial: Boolean = false,
): CanonicalConfig =
    previewPickerSave(
        canonicalBaseForSave(debug, snapshot).toCore(),
        debug,
        selfPkg,
        selections.associate { selection ->
            selection.packageName to
                CanonicalApp(
                    java = selection.java,
                    javaHooks = selection.javaHooks,
                    native = NativeRole(selection.native, selection.nativeOverrides),
                    appHiding = selection.appHiding,
                    ports = selection.ports,
                    portPolicy = selection.portPolicy,
                ).toCore()
        },
        autoHideSignals.toCoreSignals(),
        partial,
    ).toCanonical()

/**
 * [partial] is true when the picker's app-list scan (see
 * [PackageInventory.partial]) was partial — some profile other than user 0
 * didn't scan this run. [signals] then covers only the packages the picker
 * could see, so a package auto-hidden in a previous save but absent from
 * [signals] gets its `autoHiddenPackages` membership preserved (decision A:
 * a VPN app that lives only in an un-scanned profile must not get un-hidden
 * by a partial-scan Save) instead of being recomputed away.
 */
internal fun applyAutoHiddenPackages(
    config: CanonicalConfig,
    selfPkg: String,
    signals: Collection<AppAutoHideSignal>,
    partial: Boolean = false,
): CanonicalConfig = previewAutoHidden(config.toCore(), selfPkg, signals.toCoreSignals(), partial).toCanonical()

/**
 * Whether re-materializing the auto-hidden set against fresh [signals] would
 * change the set persisted for [config] — i.e. whether the startup / Refresh
 * reconcile needs to write. The reconcile write-guard: a newly-installed VPN
 * app that isn't in `autoHiddenPackages` yet flips this true; an unchanged set
 * keeps it false so the reconcile is a no-op.
 */
internal fun autoHiddenPackagesNeedReconcile(
    config: CanonicalConfig,
    selfPkg: String,
    signals: Collection<AppAutoHideSignal>,
): Boolean =
    applyAutoHiddenPackages(config, selfPkg, signals).settings.autoHiddenPackages !=
        config.settings.autoHiddenPackages

internal fun manualHiddenPackages(
    config: CanonicalConfig,
    selfPkg: String,
): Set<String> = previewManualHidden(config.toCore(), selfPkg).toSet()

internal fun updateManualHiddenPackages(
    config: CanonicalConfig,
    selfPkg: String,
    visiblePackages: Set<String>,
    selectedManualHiddenPackages: Set<String>,
    signals: Collection<AppAutoHideSignal>,
): CanonicalConfig =
    previewManualHiddenUpdate(
        config.toCore(),
        selfPkg,
        visiblePackages.toList(),
        selectedManualHiddenPackages.toList(),
        signals.toCoreSignals(),
    ).toCanonical()

private fun canonicalBaseForSave(
    debug: Boolean,
    snapshot: TargetsSnapshot?,
): CanonicalConfig =
    when {
        snapshot?.canonicalConfig != null -> {
            snapshot.canonicalConfig.copy(
                debug = debug,
                debugSwitch = snapshot.canonicalConfig.debugSwitch,
            )
        }

        snapshot != null -> {
            buildCanonicalConfigFromTargetsSnapshot(snapshot, debug = debug)
        }

        else -> {
            CanonicalConfig(debug = debug)
        }
    }

/** Row selection enables every installed protection layer; individual chips remain independent. */
internal fun toggleAllProtection(
    app: AppEntry,
    targets: TargetsSnapshot,
): AppEntry {
    val selected = !app.anySelected
    return app.copy(
        java = selected,
        javaHooks = null,
        native = targets.nativeAvailable && selected,
        nativeOverrides = NativeHookOverrides(),
        appHiding = selected,
        ports = selected,
    )
}
