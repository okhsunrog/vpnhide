package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.DashboardCache

/**
 * The one entry point of an explicit re-check ("the user asked to check again"):
 * refresh the app-VPN observation, queue one fresh suite, re-read the Dashboard's
 * root facts. Every Retry, pull-to-refresh and post-reset re-check goes through
 * here; no cache refresh requests a run on its own, and no other code path
 * requests an explicit run.
 */
internal fun retryDiagnosticsAndDashboard(
    context: Context,
    selfNeedsRestart: Boolean,
) {
    RoutingGateCache.refresh(context, selfNeedsRestart)
    DiagnosticsCache.retry(context, selfNeedsRestart)
    DashboardCache.refresh(context, selfNeedsRestart)
}
