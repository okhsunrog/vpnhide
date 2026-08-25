package dev.okhsunrog.vpnhide.picker

import android.content.res.Resources
import dev.okhsunrog.vpnhide.R

internal fun nativeSelectionChangeError(
    current: List<AppEntry>,
    candidate: List<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
    resources: Resources,
): String? {
    val currentUsage = nativeCapacityUsage(current, targets, selfPkg)
    val candidateUsage = nativeCapacityUsage(candidate, targets, selfPkg)
    return nativeCapacityIncreaseViolation(currentUsage, candidateUsage)?.run {
        nativeCapacityMessage(resources)
    }
}

internal fun nativeSelectionSaveError(
    entries: List<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
    resources: Resources,
): String? =
    nativeCapacityUsage(entries, targets, selfPkg)
        .takeIf { it.overflow > 0 }
        ?.run { nativeCapacityMessage(resources) }

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

private fun nativeCapacityUsage(
    entries: Collection<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
): NativeTargetCapacityUsage {
    val existingNativePackages =
        targets.canonicalConfig
            ?.apps
            ?.filterValues { it.native.enabled }
            ?.keys
            ?: targets.nativeTargets
    return nativeTargetCapacityUsage(
        selfPkg = selfPkg,
        selections = entries.map(AppEntry::toRoleSelection),
        existingNativePackages = existingNativePackages,
        packageUids = targets.packageUids,
    )
}

private fun nativeCapacityMessage(resources: Resources): String =
    resources.getString(
        R.string.native_target_capacity_reached,
        NATIVE_USER_UID_CAPACITY,
    )
