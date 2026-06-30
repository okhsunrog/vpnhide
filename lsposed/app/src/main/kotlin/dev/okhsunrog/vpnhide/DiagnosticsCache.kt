package dev.okhsunrog.vpnhide

import android.content.Context
import android.net.ConnectivityManager
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.coroutines.coroutineContext

/**
 * Cache for `runAllChecks` results.
 *
 * Diagnostics answer one question: *do the hooks work for this app
 * process right now?* The hooks themselves are fixed at process
 * creation time — kmod loads at boot, LSPosed injects into
 * system_server at its boot, Zygisk hooks fire at zygote fork —
 * so a run's result is valid for the entire lifetime of this app
 * process. Re-running every tab switch is pure waste.
 *
 * State machine:
 * - [State.NotRun] — fresh, nothing attempted yet.
 * - [State.Running] — a run is in flight.
 * - [State.VpnOff] — last run aborted because no active VPN was
 *   detected. User gets a "turn on VPN, then retry" banner.
 * - [State.Failed] — last run threw (root dropped, shell exec failure). VPN may
 *   well be on; the user gets a "diagnostics failed, retry" banner — distinct
 *   from [State.VpnOff] so an active-VPN user isn't told their VPN is off.
 * - [State.Ready] — at least the fast phase is captured; [State.Ready.complete]
 *   flips to true when the slow Java probes have filled in too. Dashboard waits
 *   for the complete result, while Diagnostics can show the fast result first.
 *
 * Once a complete [State.Ready] is reached, [run] becomes a no-op — results
 * don't change mid-process. The only path back to "please retry" is killing
 * the process (a new launch starts with a fresh cache).
 */
internal object DiagnosticsCache {
    sealed interface State {
        data object NotRun : State

        data object Running : State

        data object VpnOff : State

        data object Failed : State

        data class Ready(
            val results: CheckResults,
            // false after the fast core phase (native + VPN-presence Java) —
            // enough for early Diagnostics UI; true once the slow Java probes
            // (push callback etc.) have filled in too.
            val complete: Boolean,
        ) : State
    }

    private val _state = MutableStateFlow<State>(State.NotRun)
    val state: StateFlow<State> = _state.asStateFlow()

    private var inflight: Job? = null

    // Owns runs kicked off from a non-UI caller (the Dashboard's protection
    // summary), so they survive even if no screen scope is active.
    private val cacheScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    /** Start a run if one isn't already in flight and we don't have a
     * completed result yet. Idempotent — safe to call from both
     * Dashboard and Diagnostics screens on every composition.
     */
    @Synchronized
    fun run(
        scope: CoroutineScope,
        context: Context,
    ) {
        val current = _state.value
        when (current) {
            is State.Ready -> {
                if (current.complete) return
            }

            State.Running -> {
                // Only bail if a run is genuinely still in flight. If the
                // launching scope was cancelled mid-run, the state stays
                // Running but the job is dead — fall through and relaunch so
                // Diagnostics/Dashboard don't wedge on a stale Running forever.
                if (inflight?.isActive == true) return
            }

            State.NotRun, State.VpnOff, State.Failed -> { /* proceed */ }
        }
        if (inflight?.isActive == true) return
        inflight = scope.launch { doRun(context.applicationContext) }
    }

    /** Used by the retry button in the "VPN off" / "failed" banners — a readable
     * alias for [run] at the call site (the [run] guard already permits a re-run
     * from NotRun, VpnOff, and Failed).
     */
    fun retry(
        scope: CoroutineScope,
        context: Context,
    ) = run(scope, context)

    /**
     * Suspend until the full Diagnostics result is available. Dashboard uses
     * this path so the top-level "OK" state is backed by every protection
     * probe shown in Settings → Detailed diagnostics, including the slow push-callback
     * and route/proxy Java checks.
     */
    suspend fun awaitFullResults(context: Context): CheckResults? {
        run(cacheScope, context)
        val terminal =
            state.first { it is State.VpnOff || it is State.Failed || (it is State.Ready && it.complete) }
        return (terminal as? State.Ready)?.results
    }

    private suspend fun doRun(appContext: Context) {
        _state.value = State.Running
        try {
            StartupTrace.mark("diagnostics_cache_start")
            val vpnActive = withContext(Dispatchers.IO) { isVpnActive() }
            if (!vpnActive) {
                _state.value = State.VpnOff
                StartupTrace.mark("diagnostics_cache_vpn_off")
                return
            }
            val cm = appContext.getSystemService(ConnectivityManager::class.java)
            // Phase 1 (fast): native + VPN-presence Java probes. Publish
            // immediately so Settings → Detailed diagnostics can show progress without
            // waiting for the slow phase below.
            val core = withContext(Dispatchers.IO) { runCoreChecks(cm, appContext) }
            _state.value = State.Ready(core, complete = false)
            StartupTrace.mark("diagnostics_cache_core_done")
            // Phase 2 (slow): remaining Java probes, incl. the push callback
            // that blocks for up to 3s. Dashboard waits for this full result.
            val extraJava = withContext(Dispatchers.IO) { runExtraJavaChecks(cm, appContext) }
            _state.value = State.Ready(core.copy(extraJava = extraJava), complete = true)
            StartupTrace.mark("diagnostics_cache_done")
        } catch (e: CancellationException) {
            // A cancelled job (e.g. the screen left) must propagate so
            // structured concurrency unwinds — never get reinterpreted as a
            // VpnOff result. If we were cancelled before publishing a result,
            // reset Running back to NotRun so a later run() can relaunch.
            resetRunningIfStillOurs(coroutineContext.job)
            throw e
        } catch (e: Exception) {
            // A real failure (root dropped, shell exec failure) — distinct from
            // VpnOff so an active-VPN user isn't wrongly told their VPN is off.
            // Both states offer a retry; these causes are usually transient.
            _state.value = State.Failed
            StartupTrace.mark("diagnostics_cache_failed")
            VpnHideLog.w("VpnHide-Diag", "runAllChecks failed: ${e.message}")
        }
    }

    /**
     * Reset a stranded [State.Running] back to [State.NotRun] on cancellation —
     * but only if [self] is still the current [inflight] job.
     *
     * [doRun]'s catch runs asynchronously on a dispatcher after [run] has
     * already returned and released the monitor. Between a run's cancellation
     * (its job goes `!isActive`) and its catch actually landing, a later [run]
     * can fall through the dead-job guard and relaunch — setting `inflight` to
     * the new job and `_state = Running` for *its* run. An unconditional reset
     * from the cancelled run would then clobber the live run's state with a
     * spurious NotRun. Guarding on job identity (and doing the read-modify-write
     * under the same monitor [run] uses) keeps only our own cancellation able to
     * reset, atomically against a concurrent relaunch.
     */
    @Synchronized
    private fun resetRunningIfStillOurs(self: Job?) {
        if (inflight === self && _state.value is State.Running) {
            _state.value = State.NotRun
        }
    }
}
