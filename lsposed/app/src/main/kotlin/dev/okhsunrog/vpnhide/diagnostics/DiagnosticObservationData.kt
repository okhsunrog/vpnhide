package dev.okhsunrog.vpnhide.diagnostics

internal fun isTerminalDiagnosticState(state: DiagnosticsCache.State): Boolean =
    state is DiagnosticsCache.State.Blocked || state is DiagnosticsCache.State.Failed ||
        (state is DiagnosticsCache.State.Ready && state.complete)
