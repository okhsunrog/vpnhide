package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first

/** A dependent observation may initialize/join a run, but cannot implicitly retry a terminal result. */
internal suspend fun awaitDiagnosticObservation(
    state: StateFlow<DiagnosticsCache.State>,
    ensureStarted: () -> Unit,
): DiagnosticsCache.State {
    if (!isTerminalDiagnosticState(state.value)) ensureStarted()
    return state.first(::isTerminalDiagnosticState)
}

/** A refresh temporarily makes readiness unknown; that is not another transition into the VPN. */
internal fun Flow<DiagnosticGate?>.routedTransitions(): Flow<DiagnosticGate> =
    filterNotNull().distinctUntilChanged().filter { it == DiagnosticGate.ROUTED }
