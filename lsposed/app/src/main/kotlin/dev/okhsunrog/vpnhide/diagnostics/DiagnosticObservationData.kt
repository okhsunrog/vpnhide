package dev.okhsunrog.vpnhide.diagnostics

internal fun isTerminalDiagnosticState(state: DiagnosticsCache.State): Boolean =
    state is DiagnosticsCache.State.Blocked || state is DiagnosticsCache.State.Failed ||
        (state is DiagnosticsCache.State.Ready && state.complete)

/** One finished attempt in the legacy vocabulary; blocked, failed and measured stay distinct. */
internal fun projectDiagnosticAttempt(
    attempt: DiagnosticAttempt,
    results: CheckResults?,
): DiagnosticsCache.State =
    when (attempt.outcome) {
        RunOutcome.Completed -> {
            results?.let { DiagnosticsCache.State.Ready(it, complete = true) } ?: DiagnosticsCache.State.Failed
        }

        RunOutcome.NotStarted -> {
            attempt.eligibility?.blockedGate()?.let { DiagnosticsCache.State.Blocked(it) }
                ?: DiagnosticsCache.State.Failed
        }

        RunOutcome.Interrupted, RunOutcome.Failed -> {
            DiagnosticsCache.State.Failed
        }
    }

/** The active run wins while present: partial evidence shows as an incomplete Ready, everything earlier as Running. */
internal fun projectDiagnosticState(view: DiagnosticRunView): DiagnosticsCache.State {
    if (view.core.active != null) {
        return view.activeResults?.let { DiagnosticsCache.State.Ready(it, complete = false) } ?: DiagnosticsCache.State.Running
    }
    val attempt = view.core.lastAttempt ?: return DiagnosticsCache.State.NotRun
    return projectDiagnosticAttempt(attempt, view.attemptResults[attempt.id])
}

/** The existing retry policy: a completed suite is reused; a blocked, failed or absent one may be requested again. */
internal fun diagnosticRetryAllowed(state: DiagnosticRunState): Boolean =
    state.active != null || state.lastAttempt?.outcome != RunOutcome.Completed
