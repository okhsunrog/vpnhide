package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import kotlinx.serialization.Serializable

// The native layer is exactly one of these at runtime (docs/storage.md §4.3).
@Serializable
internal enum class NativeBackendId { Kmod, Kpm, Zygisk }

@Serializable
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
@Serializable
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
    currentBootId: String = sections["current_boot_id"].orEmpty(),
    kpmLoadStatus: KpmLoadStatus = parseKpmLoadStatus(sections["kpm_load_status"].orEmpty()),
): NativeBackendStates =
    NativeBackendStates(
        kmod = detectKmodModule(sections),
        kpm = detectKpmModule(sections, kpmLoadStatus, currentBootId),
        zygisk =
            detectZygiskModule(
                sections = sections,
                zygiskStatusRaw = sections["zygisk_status"].orEmpty(),
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

/**
 * An active kernel backend that did not install every hook it owns.
 *
 * The backend resolves each target by name when it loads, so a function a
 * vendor kernel renamed (Clang CFI/LTO manglings) or never exported is skipped
 * — the module reports `PARTIAL_HOOKS` and the vectors behind those hooks stay
 * visible. Without this the dashboard shows a plain green "active" card next to
 * a red leak in Diagnostics, which reads as the app contradicting itself.
 *
 * Null unless there is something to say: a non-kernel/inactive backend, an
 * unread status ([reportedHooks] empty), or a complete install all return null.
 */
internal data class PartialHookGap(
    val installed: Int,
    val expected: Int,
    val missing: List<HookIds.Hook>,
)

internal fun partialHookGap(
    backend: DisplayNativeBackend,
    reportedHooks: Set<HookIds.Hook>,
): PartialHookGap? {
    if (!moduleActive(backend.state)) return null
    val missing = missingBackendHooks(backend.id, reportedHooks)
    if (missing.isEmpty()) return null
    return PartialHookGap(
        installed = KERNEL_HOOKS.size - missing.size,
        expected = KERNEL_HOOKS.size,
        missing = missing.sortedBy { it.id },
    )
}
