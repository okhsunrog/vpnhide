package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import dev.okhsunrog.vpnhide.DashboardCache

/** One user retry: refresh VPN state, queue one fresh suite, then rederive Dashboard. */
internal fun retryDiagnosticsAndDashboard(
    context: Context,
    selfNeedsRestart: Boolean,
) {
    RoutingGateCache.refresh(context, selfNeedsRestart)
    DiagnosticsCache.retry(context, selfNeedsRestart)
    DashboardCache.refresh(context, selfNeedsRestart, runBeforeRefresh = false)
}
