package dev.okhsunrog.vpnhide.picker

import android.content.res.Resources
import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.toCore
import uniffi.vpnhide_core_ffi.CoreNativeBackend
import uniffi.vpnhide_core_ffi.CoreNativePolicyUsage
import uniffi.vpnhide_core_ffi.nativePolicyIsReduction
import uniffi.vpnhide_core_ffi.previewNativePolicy

internal fun nativeSelectionChangeError(
    current: List<AppEntry>,
    candidate: List<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
    resources: Resources,
): String? {
    val before = nativePickerConfig(current, targets, selfPkg)
    val after = nativePickerConfig(candidate, targets, selfPkg)
    if (nativePolicyIsReduction(before.toCore(), after.toCore(), targets.selectedNativeBackendId?.toPolicyBackend())) return null
    val currentUsage = nativeCapacityUsage(current, targets, selfPkg)
    val candidateUsage = nativeCapacityUsage(candidate, targets, selfPkg)
    if (currentUsage == null || candidateUsage == null) return resources.getString(R.string.native_capacity_unavailable)
    return nativeCapacityIncreaseViolation(currentUsage, candidateUsage)?.message(resources)
}

internal fun nativeSelectionSaveError(
    entries: List<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
    resources: Resources,
): String? = nativeCapacityUsage(entries, targets, selfPkg)?.takeUnless(::nativePolicyFits)?.message(resources)

/** UI interaction rule only; all UID, hook, profile and byte accounting comes from Rust. */
internal fun nativeCapacityIncreaseViolation(
    current: CoreNativePolicyUsage,
    candidate: CoreNativePolicyUsage,
): CoreNativePolicyUsage? =
    candidate.takeIf {
        (it.targetUids > it.targetCapacity && it.targetUids > current.targetUids) ||
            (it.wireBytes > it.wireCapacity && it.wireBytes > current.wireBytes)
    }

internal fun nativePolicyFits(usage: CoreNativePolicyUsage): Boolean =
    usage.targetUids <= usage.targetCapacity && usage.wireBytes <= usage.wireCapacity

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

internal fun nativeCapacityUsage(
    entries: Collection<AppEntry>,
    targets: TargetsSnapshot,
    selfPkg: String,
): CoreNativePolicyUsage? {
    if (!targets.nativeInventoryComplete) return null
    val backend = targets.selectedNativeBackendId?.toPolicyBackend() ?: return null
    val config = nativePickerConfig(entries, targets, selfPkg)
    return previewNativePolicy(config.toCore(), backend, targets.packageUids.mapValues { (_, uids) -> uids.map(Int::toUInt) })
}

private fun NativeBackendId.toPolicyBackend(): CoreNativeBackend =
    when (this) {
        NativeBackendId.Kmod -> CoreNativeBackend.KMOD
        NativeBackendId.Builtin -> CoreNativeBackend.BUILTIN
        NativeBackendId.Kpm -> CoreNativeBackend.KPM
        NativeBackendId.Zygisk -> CoreNativeBackend.ZYGISK
    }

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
