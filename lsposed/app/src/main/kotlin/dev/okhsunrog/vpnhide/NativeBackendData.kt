package dev.okhsunrog.vpnhide

// The native layer is exactly one of these at runtime (protocol §1.5).
internal enum class NativeBackendId { Kmod, Kpm, Zygisk }

internal data class NativeBackendStates(
    val kmod: ModuleState,
    val kpm: ModuleState,
    val zygisk: ModuleState,
) {
    val activeId: NativeBackendId?
        get() = activeNativeBackendId(this)
}

// Single source of truth for "is anything installed at all" — reused by both
// the install-recommendation gate (noneInstalled) and the missing-native
// error (anyInstalled) so the two checks can't independently drift out of
// sync the way two hand-written `kmod is X || kpm is X || zygisk is X`
// expressions could.
internal val NativeBackendStates.anyInstalled: Boolean
    get() = kmod is ModuleState.Installed || kpm is ModuleState.Installed || zygisk is ModuleState.Installed

internal val NativeBackendStates.noneInstalled: Boolean
    get() = !anyInstalled

/**
 * The one native backend to surface on the dashboard. This is display state:
 * an active backend wins, otherwise the highest-priority installed backend is
 * shown so the user can see what is installed but not active yet.
 */
internal data class DisplayNativeBackend(
    val id: NativeBackendId?,
    val state: ModuleState,
)

private val NATIVE_BACKEND_PRIORITY =
    listOf(
        NativeBackendId.Kmod,
        NativeBackendId.Kpm,
        NativeBackendId.Zygisk,
    )

internal fun detectNativeBackendStates(
    sections: Map<String, String>,
    selfPkg: String = APP_PACKAGE_NAME,
    currentBootId: String = sections["current_boot_id"].orEmpty(),
): NativeBackendStates =
    NativeBackendStates(
        kmod = detectKmodModule(sections, selfPkg),
        kpm = detectKpmModule(sections, selfPkg, currentBootId),
        zygisk =
            detectZygiskModule(
                sections = sections,
                zygiskStatusRaw = sections["zygisk_status"].orEmpty(),
                selfPkg = selfPkg,
                currentBootId = currentBootId,
            ),
    )

internal fun activeNativeBackendId(states: NativeBackendStates): NativeBackendId? =
    prioritizedNativeBackends(states).firstOrNull { moduleActive(it.second) }?.first

internal fun displayNativeBackend(states: NativeBackendStates): DisplayNativeBackend {
    val ordered = prioritizedNativeBackends(states)
    val installed = ordered.filter { it.second is ModuleState.Installed }
    if (installed.isEmpty()) return DisplayNativeBackend(null, ModuleState.NotInstalled)
    val displayed = installed.firstOrNull { moduleActive(it.second) } ?: installed.first()
    return DisplayNativeBackend(displayed.first, displayed.second)
}

private fun prioritizedNativeBackends(states: NativeBackendStates): List<Pair<NativeBackendId, ModuleState>> =
    NATIVE_BACKEND_PRIORITY.map { id ->
        id to
            when (id) {
                NativeBackendId.Kmod -> states.kmod
                NativeBackendId.Kpm -> states.kpm
                NativeBackendId.Zygisk -> states.zygisk
            }
    }
