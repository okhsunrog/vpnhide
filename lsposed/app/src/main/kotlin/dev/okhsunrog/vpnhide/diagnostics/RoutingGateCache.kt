package dev.okhsunrog.vpnhide.diagnostics

import android.os.SystemClock
import dev.okhsunrog.vpnhide.ContextStateCache
import dev.okhsunrog.vpnhide.DashboardCache
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationRequest
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.StateCache
import dev.okhsunrog.vpnhide.debug.captureGateFrom
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

/** Foreground-return re-probes are coalesced to at most one per this window. */
private const val RESUME_REFRESH_THROTTLE_MS = 4_000L

/**
 * Shared, app-scoped source for the routing gate — is a VPN up, and is THIS
 * app routed through it. This used to be computed independently in three
 * places (the debug-export sheet, the logcat-record card, and
 * [DiagnosticsCache]'s own gate fold), so a re-check in one place never
 * updated the others and each surface could disagree about the current
 * state. One [StateCache] of [DiagnosticGate] now backs all of them:
 * [ensureLoaded] / [refresh] mirror [DashboardCache]'s stash-then-delegate
 * shape, and [gate] is the one [StateFlow] every screen collects.
 *
 * [load] always folds through [captureGateFrom] (VPN-iface read off the root
 * snapshot + [GroundTruthProbe.selfRoutedThroughVpn]) — the same pure probe
 * the export/logcat gate used to call directly — so the value here is never
 * a looser, framework-only approximation.
 */
internal object RoutingGateCache : ContextStateCache<DiagnosticGate>(
    traceName = "routing_gate",
    logTag = LogTags.DIAG,
    source = RootSnapshotCache.dependency,
) {
    val gate: StateFlow<DiagnosticGate?> get() = current

    // Monotonic timestamp of the last actual (re)load — stamped in load() so it counts
    // every refresh source (VpnTransportWatcher, manual re-check, config write). Used to
    // throttle the belt-and-suspenders foreground-return re-probe below.
    @Volatile private var lastLoadAtMs: Long = 0L

    /**
     * Re-probe the gate on a foreground return, but at most once per
     * [RESUME_REFRESH_THROTTLE_MS] — a cheap safety net for aggressive-OEM freezers
     * that may delay or drop the background [VpnTransportWatcher] callback. Because
     * the throttle keys off [load]'s stamp (every refresh source), a resume right
     * after the watcher already refreshed is a no-op. Safe to call on every ON_RESUME.
     *
     * Deliberately reuses the application-only [inputs] (set by
     * the UI/startup [ensureLoaded]) rather than taking them from the caller: a
     * lifecycle observer captures its composable's vars once and would otherwise pass a
     * stale (e.g. still-null → false) `selfNeedsRestart`, clobbering the real one. No-op
     * until the cache has been initialized at least once.
     */
    fun refreshIfStale(
        scope: CoroutineScope,
        throttleMs: Long = RESUME_REFRESH_THROTTLE_MS,
    ) {
        if (!ready) return
        if (SystemClock.elapsedRealtime() - lastLoadAtMs < throttleMs) return
        forceRefresh(scope)
    }

    // Always on IO: captureGateFrom runs the blocking `su` self-routing probe, so a
    // caller that awaits a refresh from the main dispatcher (e.g. a card's
    // rememberCaptureGate scrolling into view) must not run it on the UI thread. The
    // old measureCaptureGate wrapped this; keep it wrapped here so no caller has to.
    override suspend fun load(
        @Suppress("UNUSED_PARAMETER") request: ObservationRequest,
    ): DiagnosticGate =
        withContext(Dispatchers.IO) {
            lastLoadAtMs = SystemClock.elapsedRealtime()
            val (context, selfNeedsRestart) = requireNotNull(inputs)
            val snapshot = RootSnapshotCache.getOrLoad()
            captureGateFrom(snapshot, context, selfNeedsRestart)
        }
}
