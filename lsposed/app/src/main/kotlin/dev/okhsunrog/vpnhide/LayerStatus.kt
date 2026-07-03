package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds

/**
 * Per-layer backend health for a dashboard tile. Presence (Absent / Inactive)
 * is decided *before* the checks, so an unloaded backend can never render as a
 * leak or a verdict — it just reads "not active". This is the fix for the old
 * "Partial" that a not-loaded backend used to show from SELinux-only passes.
 */
sealed interface LayerStatus {
    /** No backend module installed for this layer. */
    data object Absent : LayerStatus

    /** Installed but not loaded this boot (needs a reboot / manager toggle). */
    data object Inactive : LayerStatus

    /** Active and measured. [hidden] = vectors the backend provably hid;
     * [leaks] = vectors it owns that still leak. */
    data class Active(
        val hidden: Int,
        val leaks: Int,
    ) : LayerStatus
}

enum class Verdict { Ok, Partial, Broken }

/**
 * Ok = nothing owned leaks. Partial = the backend hides some but an owned vector
 * still leaks (works, has a gap). Broken = active but hid nothing while leaking
 * (loaded yet dead). [hidden] must be a *measurement* (root differential), never
 * inferred from a clean probe — else Partial and Broken are indistinguishable.
 */
val LayerStatus.Active.verdict: Verdict
    get() =
        when {
            leaks == 0 -> Verdict.Ok
            hidden > 0 -> Verdict.Partial
            else -> Verdict.Broken
        }

private fun NativeCheckSpec.hasHookIn(mask: Int): Boolean = expectedHooks.any { (1 shl it.id) and mask != 0 }

/**
 * Native tile = health of the active backend, judged **only on vectors it owns**
 * (has a hook for). A leak on a not-owned vector (e.g. /proc/net/dev under a
 * kernel backend — no kernel hook exists) is out of scope for the tile; it
 * surfaces via the hero instead. Kernel backends (kmod/KPM) own kernel-hook
 * vectors; Zygisk owns zygisk-hook vectors.
 */
internal fun summarizeNativeLayer(
    backend: DisplayNativeBackend,
    outcomes: Map<String, CheckOutcome>,
): LayerStatus {
    if (backend.state is ModuleState.NotInstalled) return LayerStatus.Absent
    if (!moduleActive(backend.state)) return LayerStatus.Inactive
    val ownMask =
        when (backend.id) {
            NativeBackendId.Kmod, NativeBackendId.Kpm -> HookIds.KERNEL_HOOK_MASK
            NativeBackendId.Zygisk -> HookIds.ZYGISK_HOOK_MASK
            null -> HookIds.KERNEL_HOOK_MASK
        }
    val ownedIds = NATIVE_CHECKS.filter { it.hasHookIn(ownMask) }.map { it.id }.toSet()
    val hidden = outcomes.values.count { it is CheckOutcome.HiddenByBackend }
    val leaks = outcomes.count { (id, outcome) -> outcome is CheckOutcome.Leak && id in ownedIds }
    return LayerStatus.Active(hidden = hidden, leaks = leaks)
}

/**
 * Java tile from the LSPosed hook state + the Java check results. Java probes are
 * framework IPC with no root differential, so a clean result is taken as
 * hidden-by-LSPosed; a couple of failing probes read as Partial ("leaking n"),
 * not a blanket "not working".
 */
internal fun summarizeJavaLayer(
    lsposedActive: Boolean,
    javaChecks: List<CheckResult>,
): LayerStatus {
    if (!lsposedActive) return LayerStatus.Inactive
    // Same shape as the native tile: hidden/leaks read off the who-hid-it outcome
    // (attached by withJavaOutcomes), so Partial vs Broken is a measurement.
    return LayerStatus.Active(
        hidden = javaChecks.count { it.outcome is CheckOutcome.HiddenByBackend },
        leaks = javaChecks.count { it.outcome is CheckOutcome.Leak },
    )
}

/** Native leaks on vectors the active backend does NOT own — surfaced via the
 * hero (a warning), not the tile. Zero when the backend covers everything that
 * leaks, or when SELinux is masking (those are hidden_selinux, not leaks). */
internal fun unownedNativeLeaks(
    backend: DisplayNativeBackend,
    outcomes: Map<String, CheckOutcome>,
): Int {
    if (backend.state !is ModuleState.Installed || !moduleActive(backend.state)) return 0
    val ownMask =
        when (backend.id) {
            NativeBackendId.Zygisk -> HookIds.ZYGISK_HOOK_MASK
            else -> HookIds.KERNEL_HOOK_MASK
        }
    val ownedIds = NATIVE_CHECKS.filter { it.hasHookIn(ownMask) }.map { it.id }.toSet()
    return outcomes.count { (id, outcome) -> outcome is CheckOutcome.Leak && id !in ownedIds }
}
