package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.net.ConnectivityManager
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.debug.captureGateFrom
import dev.okhsunrog.vpnhide.startup.StartupTrace
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
 * - [State.Blocked] — the gate stopped the run (VPN off, or this app split-tunnelled
 *   out); carries the [DiagnosticGate] so the banner explains which.
 * - [State.Failed] — last run threw (root dropped, shell exec failure). VPN may
 *   well be on; the user gets a "diagnostics failed, retry" banner — distinct
 *   from a [State.Blocked] VPN-off gate so an active-VPN user isn't told their VPN is off.
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

        // The gate blocked the run: no active VPN, or a VPN is up but this app is
        // split-tunnelled out of it (nothing to hide from us). Carries the shared
        // [DiagnosticGate] so every consumer speaks one vocabulary and no one
        // re-folds the reason. Never [DiagnosticGate.ROUTED] — that becomes [Ready].
        data class Blocked(
            val gate: DiagnosticGate,
        ) : State {
            init {
                require(gate != DiagnosticGate.ROUTED) { "Blocked gate must not be ROUTED" }
            }
        }

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

    // Whether this app's own hooks need a reboot to apply (it was just added as a
    // target). Process-constant, so it is sticky-OR: the first run() call sets it and
    // no later caller can clear it. When set, a run measures nothing (the hooks aren't
    // in this process), so the cache parks at Blocked(NEEDS_RESTART) and never probes.
    @Volatile private var restartPending = false

    /** Start a run if one isn't already in flight and we don't have a
     * completed result yet. Idempotent — safe to call from both
     * Dashboard and Diagnostics screens on every composition.
     *
     * [selfNeedsRestart] is the shared gate signal: once any caller reports true the
     * cache stays at Blocked(NEEDS_RESTART) (a run would be meaningless), so a caller
     * that doesn't know it — the agent bridge — can safely pass false.
     */
    @Synchronized
    fun run(
        scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        restartPending = restartPending || selfNeedsRestart
        if (restartPending) {
            // Publish the gate synchronously and never launch doRun — this keeps the
            // Job/cancellation machinery reserved for the real run. selfNeedsRestart is
            // process-constant, so this is decided on the first run() and never flips
            // out from under an in-flight run.
            _state.value = State.Blocked(DiagnosticGate.NEEDS_RESTART)
            return
        }
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

            State.NotRun, is State.Blocked, State.Failed -> { /* proceed */ }
        }
        if (inflight?.isActive == true) return
        inflight = scope.launch { doRun(context.applicationContext) }
    }

    /** Used by the retry button in the "VPN off" / "failed" banners — a readable
     * alias for [run] at the call site (the [run] guard already permits a re-run
     * from NotRun, Blocked, and Failed).
     */
    fun retry(
        scope: CoroutineScope,
        context: Context,
        selfNeedsRestart: Boolean,
    ) = run(scope, context, selfNeedsRestart)

    /**
     * Suspend until the full Diagnostics result is available. Dashboard uses
     * this path so the top-level "OK" state is backed by every protection
     * probe shown in Settings → Detailed diagnostics, including the slow push-callback
     * and route/NetworkInfo Java checks.
     */
    suspend fun awaitTerminal(
        context: Context,
        selfNeedsRestart: Boolean,
    ): State {
        run(cacheScope, context, selfNeedsRestart)
        return state.first {
            it is State.Blocked || it is State.Failed || (it is State.Ready && it.complete)
        }
    }

    /** The complete check results, or null when the terminal state carried no
     * measurement (blocked or failed). Callers that need to distinguish *why*
     * there are no results should use [awaitTerminal] — reading [state] again
     * after a null here would race a subsequent run. */
    suspend fun awaitFullResults(
        context: Context,
        selfNeedsRestart: Boolean,
    ): CheckResults? = (awaitTerminal(context, selfNeedsRestart) as? State.Ready)?.results

    private suspend fun doRun(appContext: Context) {
        _state.value = State.Running
        try {
            StartupTrace.mark("diagnostics_cache_start")
            // The gate now comes from the shared RoutingGateCache (folded through the
            // same resolveDiagnosticGate / captureGateFrom every other surface uses),
            // so the live path, the debug export, and the dashboard can never disagree
            // about VPN-off / self-not-routed / routed. selfNeedsRestart=false below is
            // safe — run()'s restartPending guard already ensures doRun is only reached
            // once every caller reporting into this cache has reported false.
            RoutingGateCache.ensureLoaded(cacheScope, appContext, selfNeedsRestart = false)
            withContext(Dispatchers.IO) { RoutingGateCache.refreshInPlace(force = true) }
            val gate = RoutingGateCache.gate.value
            if (gate == null) {
                throw IllegalStateException(RoutingGateCache.error.value ?: "routing gate unavailable")
            }
            if (gate != DiagnosticGate.ROUTED) {
                _state.value = State.Blocked(gate)
                StartupTrace.mark(
                    if (gate == DiagnosticGate.VPN_OFF) "diagnostics_cache_vpn_off" else "diagnostics_cache_self_not_routed",
                )
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
            // Blocked/Failed result. If we were cancelled before publishing a
            // result, reset Running back to NotRun so a later run() can relaunch.
            resetRunningIfStillOurs(coroutineContext.job)
            throw e
        } catch (e: Exception) {
            // A real failure (root dropped, shell exec failure) — distinct from a
            // VPN-off gate so an active-VPN user isn't wrongly told their VPN is off.
            // Both states offer a retry; these causes are usually transient.
            _state.value = State.Failed
            StartupTrace.mark("diagnostics_cache_failed")
            VpnHideLog.w(LogTags.DIAG, "runAllChecks failed: ${e.message}")
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
