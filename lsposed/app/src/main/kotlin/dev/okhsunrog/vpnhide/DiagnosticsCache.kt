package dev.okhsunrog.vpnhide

import android.content.Context
import android.net.ConnectivityManager
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

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
 * - [State.Ready] — results captured; exposed to both the Dashboard
 *   protection panel and the Diagnostics screen.
 *
 * Once [State.Ready] is reached, [run] becomes a no-op — results don't
 * change mid-process. The only path back to "please retry" requires
 * killing the process (new launch = fresh cache) or calling [reset]
 * (currently unused; reserved for explicit force-refresh flows, e.g.
 * "new kernel module just installed, please verify from a fresh
 * process" style prompts if we ever want them).
 */
internal object DiagnosticsCache {
    sealed interface State {
        data object NotRun : State

        data object Running : State

        data object VpnOff : State

        data class Ready(
            val results: CheckResults,
        ) : State
    }

    private val _state = MutableStateFlow<State>(State.NotRun)
    val state: StateFlow<State> = _state.asStateFlow()

    private var inflight: Job? = null

    /** Start a run if one isn't already in flight and we don't have a
     * completed result yet. Idempotent — safe to call from both
     * Dashboard and Diagnostics screens on every composition.
     */
    fun run(
        scope: CoroutineScope,
        context: Context,
    ) {
        when (_state.value) {
            is State.Ready -> {
                return
            }

            State.Running -> {
                return
            }

            State.NotRun, State.VpnOff -> { /* proceed */ }
        }
        if (inflight?.isActive == true) return
        inflight = scope.launch { doRun(context.applicationContext) }
    }

    /** Used by the retry button in the "VPN off" banner. Same as [run]
     * but bypasses the `VpnOff` short-circuit (which [run] doesn't
     * actually have — the guard above covers NotRun/VpnOff both — so
     * this is really just a named alias for readability at the
     * callsite).
     */
    fun retry(
        scope: CoroutineScope,
        context: Context,
    ) = run(scope, context)

    /** Drop any cached result. Next [run] will execute fresh.
     * Not wired to any UI at the moment — reserved for scenarios like
     * "user installed a new module and wants to force a re-run from the
     * same process". Normal refreshes never need this because results
     * are fixed for the process lifetime.
     */
    fun reset() {
        inflight?.cancel()
        _state.value = State.NotRun
    }

    private suspend fun doRun(appContext: Context) {
        _state.value = State.Running
        try {
            val vpnActive = withContext(Dispatchers.IO) { isVpnActive() }
            if (!vpnActive) {
                _state.value = State.VpnOff
                return
            }
            val results =
                withContext(Dispatchers.IO) {
                    val cm = appContext.getSystemService(ConnectivityManager::class.java)
                    runAllChecks(cm, appContext)
                }
            _state.value = State.Ready(results)
        } catch (e: Exception) {
            // Failures leave us in VpnOff so the user sees the retry UI
            // rather than a frozen spinner. Real-world causes here are
            // transient (root dropped, shell exec failure) and a retry
            // usually works.
            _state.value = State.VpnOff
            VpnHideLog.w("VpnHide-Diag", "runAllChecks failed: ${e.message}")
        }
    }
}
