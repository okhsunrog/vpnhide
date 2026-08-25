package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.DisplayNativeBackend
import dev.okhsunrog.vpnhide.ModuleState
import dev.okhsunrog.vpnhide.coveredBy
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.moduleActive
import dev.okhsunrog.vpnhide.ownedNativeHooks
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * Per-layer backend health for a dashboard tile. Presence (Absent / Inactive)
 * is decided *before* the checks, so an unloaded backend can never render as a
 * leak or a verdict — it just reads "not active". This is the fix for the old
 * "Partial" that a not-loaded backend used to show from SELinux-only passes.
 */
@Serializable
sealed interface LayerStatus {
    /** No backend module installed for this layer. */
    @Serializable
    @SerialName("absent")
    data object Absent : LayerStatus

    /** Installed but not loaded this boot (needs a reboot / manager toggle). */
    @Serializable
    @SerialName("inactive")
    data object Inactive : LayerStatus

    /** Installed and present, but the snapshot shell could not confirm liveness
     * (e.g. it lacked root to read the runtime resource). Rendered as "not
     * verified", never as a false "inactive". */
    @Serializable
    @SerialName("unverified")
    data object Unverified : LayerStatus

    /** Active and measured. [hidden] = vectors the backend provably hid;
     * [leaks] = vectors it owns that still leak. */
    @Serializable
    @SerialName("active")
    data class Active(
        val hidden: Int,
        val leaks: Int,
    ) : LayerStatus
}

@Serializable
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

/** Native-check ids the active backend owns — its checks whose expected hooks the
 * backend covers. One derivation shared by the tile summary and the unowned count. */
private fun ownedNativeCheckIds(
    backend: DisplayNativeBackend,
    installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
): Set<String> {
    val owned = ownedNativeHooks(backend.id, installedOptionalHooks)
    return NATIVE_CHECKS.filter { it.coveredBy(owned) }.map { it.id }.toSet()
}

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
    installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
): LayerStatus {
    val state = backend.state
    if (state is ModuleState.NotInstalled) return LayerStatus.Absent
    if (!moduleActive(state)) {
        // Installed but not active. If the snapshot shell couldn't authoritatively
        // read liveness (non-root → false negative on the 0600 ctl / iptables),
        // report "not verified" instead of a misleading "inactive".
        return if ((state as? ModuleState.Installed)?.runtimeCheckable == false) {
            LayerStatus.Unverified
        } else {
            LayerStatus.Inactive
        }
    }
    val ownedIds = ownedNativeCheckIds(backend, installedOptionalHooks)
    // Both counts are scoped to vectors this backend owns, so hidden and leaks
    // describe the same vector set — a cross-backend hidden (only possible if the
    // one-active-backend invariant ever breaks) can't mask an owned Broken verdict.
    val hidden = outcomes.count { (id, outcome) -> outcome is CheckOutcome.HiddenByBackend && id in ownedIds }
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
    // (set at construction via javaCheck), so Partial vs Broken is a measurement.
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
    installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
): Int {
    if (backend.state !is ModuleState.Installed || !moduleActive(backend.state)) return 0
    val ownedIds = ownedNativeCheckIds(backend, installedOptionalHooks)
    return outcomes.count { (id, outcome) -> outcome is CheckOutcome.Leak && id !in ownedIds }
}
