package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.DisplayNativeBackend
import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.coveredBy
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.missingBackendHooks
import dev.okhsunrog.vpnhide.ownedHooks
import dev.okhsunrog.vpnhide.ownedNativeHooks
import dev.okhsunrog.vpnhide.settings.installedNativeOptionalHooks
import kotlinx.serialization.Serializable

@Serializable
internal enum class CheckLayer { NATIVE, JAVA }

/**
 * Why the check suite did or did not run this session — the terminal gate from
 * [DiagnosticsCache]. Only [ROUTED] yields measured per-layer verdicts; the other
 * states mean "we deliberately did not measure" (VPN off, this app split-tunnelled
 * out of the VPN, or a pending self-restart), which a consumer must surface
 * instead of a misleading clean/Ok result.
 */
@Serializable
internal enum class DiagnosticGate { VPN_OFF, SELF_NOT_ROUTED, NEEDS_RESTART, ROUTED }

/**
 * Fold the three independent gate signals — a pending self-restart, VPN presence,
 * and this app's self-in-tunnel routing — into one [DiagnosticGate], most-blocking
 * first, so the dashboard, the live cache, and the debug export classify the run the
 * same way. Only [DiagnosticGate.ROUTED] means "measured; verdicts are meaningful".
 *
 * [NEEDS_RESTART] outranks [VPN_OFF]: when this app was just added as a target its
 * own hooks aren't applied to this process yet, so a run would measure nothing no
 * matter the VPN state — "reboot to apply" is the actionable step, not "turn on VPN".
 */
internal fun resolveDiagnosticGate(
    vpnActive: Boolean,
    selfRouted: Boolean?,
    selfNeedsRestart: Boolean,
): DiagnosticGate =
    when {
        selfNeedsRestart -> DiagnosticGate.NEEDS_RESTART
        !vpnActive -> DiagnosticGate.VPN_OFF
        selfRouted == false -> DiagnosticGate.SELF_NOT_ROUTED
        else -> DiagnosticGate.ROUTED
    }

/** The blocking gate to surface, or null when ROUTED (measurement is meaningful). */
internal fun DiagnosticGate.blockedOrNull(): DiagnosticGate? =
    takeIf { it == DiagnosticGate.VPN_OFF || it == DiagnosticGate.SELF_NOT_ROUTED || it == DiagnosticGate.NEEDS_RESTART }

/**
 * One fully-classified check: the root-differential [outcome] plus the evidence
 * behind it — the app-view [appDetail], the root [groundTruthDetail] (native
 * probes run as root), and, for native probes, the hooks that should cover the
 * vector and whether the active backend [owned] it.
 */
@Serializable
internal data class DiagnosticCheck(
    val id: String,
    val label: String,
    val layer: CheckLayer,
    val outcome: CheckOutcome,
    val appDetail: String,
    val groundTruthDetail: String?,
    val expectedHooks: List<HookIds.Hook>,
    val owned: Boolean,
    // Expected hooks the active kernel backend reported as NOT installed. A leak
    // here is a kernel-side gap (a renamed/absent symbol), not a misconfiguration
    // — the difference between "reinstall the module" and "your kernel does not
    // expose this function". Empty on every healthy device.
    val missingHooks: List<HookIds.Hook> = emptyList(),
)

/** Per-layer rollup: presence plus the classified checks that produced it. The
 * Ok/Partial/Broken verdict is deliberately NOT exposed here — it is only valid
 * for a measured run, so it is reachable only through the gate-checked
 * [DiagnosticReport.nativeVerdict] / [DiagnosticReport.javaVerdict]. */
@Serializable
internal data class LayerReport(
    val layer: CheckLayer,
    val backend: NativeBackendId?,
    val status: LayerStatus,
    // Leaks on vectors the active backend does NOT own (native only) — surfaced as
    // a warning, never counted against the tile verdict. Always 0 for the Java layer.
    val unownedLeaks: Int,
    val checks: List<DiagnosticCheck>,
)

/**
 * The single canonical diagnostic snapshot.
 *
 * The app used to re-derive the diagnostic verdict independently in four places
 * (dashboard tiles, the Diagnostics screen, the agent bridge, and the debug-ZIP
 * text renderers), each flattening the rich per-check attribution back down to a
 * PASS/FAIL badge. [buildDiagnosticReport] computes the whole thing **once**;
 * every consumer is a pure render of this object, so no two views can disagree
 * and the debug bundle carries exactly what the UI shows.
 */
@Serializable
internal data class DiagnosticReport(
    val gate: DiagnosticGate,
    val native: LayerReport,
    val java: LayerReport,
    // False after the fast core phase, true once the slow Java probes have filled in.
    val complete: Boolean,
) {
    /** Per-layer Ok/Partial/Broken — the ONLY way to read a report's verdict.
     * Null unless the run was actually measured ([DiagnosticGate.ROUTED]); a
     * gated report's layers carry a placeholder [LayerStatus.Active] with zero
     * counts, so returning its verdict would render a false "Ok". */
    val nativeVerdict: Verdict? get() = native.verdictFor(gate)
    val javaVerdict: Verdict? get() = java.verdictFor(gate)
}

/** Verdict of a layer, but only for a measured run — the gate is required so no
 * caller can obtain a verdict without acknowledging whether the run measured
 * anything. */
private fun LayerReport.verdictFor(gate: DiagnosticGate): Verdict? =
    if (gate == DiagnosticGate.ROUTED) (status as? LayerStatus.Active)?.verdict else null

/**
 * Fold the raw check run into the canonical [DiagnosticReport]. Pure: same inputs
 * → same report, no Android or IO dependency, so it is unit-tested directly.
 *
 * [results] is null when the gate blocked the run ([DiagnosticGate.ROUTED] is the
 * only gate that carries measurements); the layers then report presence only, and
 * [DiagnosticReport.nativeVerdict] / [DiagnosticReport.javaVerdict] return null so
 * a gated verdict can never be rendered.
 */
internal fun buildDiagnosticReport(
    gate: DiagnosticGate,
    results: CheckResults?,
    backend: DisplayNativeBackend,
    lsposedActive: Boolean,
    complete: Boolean,
    installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
): DiagnosticReport {
    val nativeChecks = nativeDiagnosticChecks(results, backend, installedOptionalHooks)
    // The per-check outcome is the single source of truth; the by-id map the layer
    // summary needs is derived here (owned spec checks only), not stored a second
    // time on CheckResults.
    val nativeOutcomes = nativeChecks.filter { it.id.isNotEmpty() }.associate { it.id to it.outcome }
    val unowned =
        if (results == null) {
            0
        } else {
            unownedNativeLeaks(backend, nativeOutcomes, installedOptionalHooks) +
                results.nativeExtra.count { it.outcome is CheckOutcome.Leak }
        }
    return DiagnosticReport(
        gate = gate,
        native =
            LayerReport(
                layer = CheckLayer.NATIVE,
                backend = backend.id,
                status = summarizeNativeLayer(backend, nativeOutcomes, installedOptionalHooks),
                unownedLeaks = unowned,
                checks = nativeChecks,
            ),
        java =
            LayerReport(
                layer = CheckLayer.JAVA,
                backend = null,
                status = summarizeJavaLayer(lsposedActive, results?.java ?: emptyList()),
                unownedLeaks = 0,
                checks = javaDiagnosticChecks(results),
            ),
        complete = complete,
    )
}

private fun nativeDiagnosticChecks(
    results: CheckResults?,
    backend: DisplayNativeBackend,
    installedOptionalHooks: Set<HookIds.Hook>,
): List<DiagnosticCheck> {
    if (results == null) return emptyList()
    val ownedHooks = ownedNativeHooks(backend.id, installedOptionalHooks)
    // For a kernel backend the caller passes the backend's whole reported hook
    // mask (FilesystemHidingData.installedNativeOptionalHooks), so the gap against
    // the family set is exactly the hooks that failed to install this boot.
    val missingHooks = missingBackendHooks(backend.id, installedOptionalHooks)
    // native and nativeExtra are built in NATIVE_CHECKS order, so a positional zip
    // is stable by construction — the spec carries the stable id + hook coverage,
    // the result carries the localized label, outcome, and root ground-truth detail.
    val owned =
        NATIVE_CHECKS.zip(results.native) { spec, cr ->
            DiagnosticCheck(
                id = spec.id,
                label = cr.name,
                layer = CheckLayer.NATIVE,
                outcome = cr.outcome,
                appDetail = cr.detail,
                groundTruthDetail = cr.groundTruthDetail,
                expectedHooks = spec.expectedHooks.toList(),
                owned = spec.coveredBy(ownedHooks),
                missingHooks = spec.expectedHooks.filter { it in missingHooks },
            )
        }
    // Java-implemented native-level probes (NetworkInterface enum): no hook
    // ownership and no root differential, so the outcome comes off the tri-state.
    val extra =
        results.nativeExtra.map { cr ->
            DiagnosticCheck(
                id = "",
                label = cr.name,
                layer = CheckLayer.NATIVE,
                outcome = cr.outcome,
                appDetail = cr.detail,
                groundTruthDetail = null,
                expectedHooks = emptyList(),
                owned = false,
            )
        }
    return owned + extra
}

private fun javaDiagnosticChecks(results: CheckResults?): List<DiagnosticCheck> =
    results
        ?.java
        ?.map { cr ->
            DiagnosticCheck(
                id = "",
                label = cr.name,
                layer = CheckLayer.JAVA,
                outcome = cr.outcome,
                appDetail = cr.detail,
                groundTruthDetail = cr.groundTruthDetail,
                expectedHooks = emptyList(),
                owned = true,
            )
        }.orEmpty()
