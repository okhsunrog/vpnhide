package dev.okhsunrog.vpnhide.picker

import android.content.res.Resources
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.toCore
import uniffi.vpnhide_core_ffi.CoreNativePickerDecision
import uniffi.vpnhide_core_ffi.CoreNativePolicyUsage
import uniffi.vpnhide_core_ffi.previewNativePickerChange
import uniffi.vpnhide_core_ffi.previewNativePickerSave

internal fun nativeSelectionChangeError(
    current: List<AppEntry>,
    candidate: List<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
    resources: Resources,
): String? {
    val before = nativePickerConfig(current, targets, selfPkg)
    val after = nativePickerConfig(candidate, targets, selfPkg)
    return when (
        val decision =
            previewNativePickerChange(
                before.toCore(),
                after.toCore(),
                targets.capacityBackends,
                targets.packageUids.mapValues { (_, uids) -> uids.map(Int::toUInt) },
                targets.nativeInventoryComplete,
            )
    ) {
        CoreNativePickerDecision.Allowed -> null
        CoreNativePickerDecision.InventoryUnavailable -> resources.getString(R.string.native_capacity_unavailable)
        is CoreNativePickerDecision.Capacity -> decision.usage.message(resources)
    }
}

internal fun nativeSelectionSaveError(
    entries: List<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
    resources: Resources,
): String? =
    when (val decision = nativePickerSaveDecision(entries, targets, selfPkg)) {
        is CoreNativePickerDecision.Capacity -> decision.usage.message(resources)

        // Save still goes through fresh Core admission when accounting is unavailable.
        CoreNativePickerDecision.Allowed, CoreNativePickerDecision.InventoryUnavailable -> null
    }

internal fun nativePickerSaveDecision(
    entries: Collection<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
): CoreNativePickerDecision =
    previewNativePickerSave(
        nativePickerConfig(entries, targets, selfPkg).toCore(),
        targets.capacityBackends,
        targets.packageUids.mapValues { (_, uids) -> uids.map(Int::toUInt) },
        targets.nativeInventoryComplete,
    )

internal fun AppEntry.toRoleSelection(): AppRoleSelection =
    AppRoleSelection(
        packageName = packageName,
        uids = uids,
        java = java,
        javaHooks = javaHooks,
        native = native,
        nativeOverrides = nativeOverrides,
        appHiding = appHiding,
        ports = ports,
        portPolicy = portPolicy,
    )

private fun nativePickerConfig(
    entries: Collection<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
) = buildCanonicalConfigForAppPickerSave(
    debug = targets.canonicalConfig?.debug ?: false,
    selfPkg = selfPkg,
    selections = entries.map(AppEntry::toRoleSelection),
    snapshot = targets,
    partial = true,
)

private fun CoreNativePolicyUsage.message(resources: Resources): String =
    if (targetUids > targetCapacity) {
        resources.getString(
            R.string.native_target_capacity_over,
            targetUids.toLong(),
            targetCapacity.toLong(),
            (targetUids - targetCapacity).toLong(),
        )
    } else {
        resources.getString(R.string.native_wire_capacity_over, wireBytes.toLong(), wireCapacity.toLong())
    }
