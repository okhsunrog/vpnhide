package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable

// The native layer is exactly one of these at runtime (docs/storage.md §4.3).
// Builtin is the in-tree (CONFIG_VPNHIDE=y) kernel backend — "kmod without the
// insmod": it owns the same kernel hooks and shares /proc/vpnhide_ctl with the
// .ko, and the two are mutually exclusive (the kernel either has the driver
// built in or loads the module).
@Serializable
internal enum class NativeBackendId { Kmod, Builtin, Kpm, Zygisk }

@Serializable
internal data class NativeBackendStates(
    val kmod: ModuleState,
    val kpm: ModuleState,
    val zygisk: ModuleState,
    val builtin: ModuleState = ModuleState.NotInstalled,
)

/** Compatibility export identity plus legacy installation metadata; Core owns runtime/setup. */
@Serializable
internal data class DisplayNativeBackend(
    val id: NativeBackendId?,
    val state: ModuleState,
)

/** Only standalone module metadata, never a liveness or backend-selection oracle. */
internal fun nativeLegacyInventory(sections: Map<String, String>): NativeBackendStates {
    fun installed(key: String): ModuleState {
        val prop = parseModuleProp(sections[key].orEmpty())
        return if (prop.installed) {
            ModuleState.Installed(
                prop.version,
                active = false,
                gkiVariant = prop.gkiVariant,
                runtimeCheckable = false,
            )
        } else {
            ModuleState.NotInstalled
        }
    }
    return NativeBackendStates(
        kmod = installed("kmod_prop"),
        kpm = installed("kpm_prop"),
        zygisk = installed("zygisk_prop"),
        builtin = installed("builtin_prop"),
    )
}

/** The selected Core implementation may be bundled and have no legacy module. */
internal fun coreDisplayNativeBackend(
    legacy: NativeBackendStates,
    snapshot: NativeStatusSnapshot,
): DisplayNativeBackend {
    val backend =
        snapshot.model
            ?.displayed
            ?.backend
            .takeIf { snapshot.current }
    val metadata =
        when (backend) {
            NativeBackendId.Kmod -> legacy.kmod
            NativeBackendId.Builtin -> legacy.builtin
            NativeBackendId.Kpm -> legacy.kpm
            NativeBackendId.Zygisk -> legacy.zygisk
            null -> ModuleState.NotInstalled
        }
    return DisplayNativeBackend(backend, metadata)
}
