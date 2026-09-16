package dev.okhsunrog.vpnhide.diagnostics

/** The existing retry policy: a completed suite is reused; a blocked, failed or absent one may be requested again. */
internal fun diagnosticRetryAllowed(state: DiagnosticRunState): Boolean =
    state.active != null || state.lastAttempt?.outcome != RunOutcome.Completed
