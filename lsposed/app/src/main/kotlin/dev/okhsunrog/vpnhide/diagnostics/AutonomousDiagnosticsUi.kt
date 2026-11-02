package dev.okhsunrog.vpnhide.diagnostics

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.StatusColors
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard

/** Wording only: every vector/outcome was evaluated by Core. No success is invented. */
@Composable
internal fun AutonomousDiagnosticStatusCard(
    snapshot: AutonomousDiagnosticSnapshot,
    onRetry: () -> Unit,
) {
    val assessment = snapshot.assessment
    val retry by DiagnosticsCache.retryState.collectAsStateWithLifecycle()
    val running = snapshot.current && assessment?.phase == "running"
    val hasLeaks = assessment?.leaked?.isNotEmpty() == true
    val title =
        when {
            snapshot.availability == AutonomousAvailability.Initializing || running -> R.string.autonomous_running
            !snapshot.current -> R.string.autonomous_unavailable
            hasLeaks -> R.string.autonomous_leaks
            assessment?.phase == "infrastructure" -> R.string.autonomous_unavailable
            else -> R.string.autonomous_incomplete
        }
    val color =
        when {
            snapshot.current && hasLeaks -> StatusColors.errorContainer()
            snapshot.current && assessment?.phase == "incomplete" -> StatusColors.warningContainer()
            else -> StatusColors.neutralContainer()
        }
    EnhancedCard(modifier = Modifier.fillMaxWidth(), color = color) {
        Column(Modifier.padding(16.dp)) {
            Text(stringResource(title), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            if (snapshot.run != null && !snapshot.current) {
                Text(stringResource(R.string.autonomous_history), style = MaterialTheme.typography.bodySmall)
            }
            assessment?.let {
                Spacer(Modifier.height(8.dp))
                Text(stringResource(R.string.autonomous_tally, it.proven.size, it.leaked.size, it.unverified.size))
                if (it.reuseBlockers.contains("loaded_generations_incomplete")) {
                    Text(stringResource(R.string.autonomous_process_coverage), style = MaterialTheme.typography.bodySmall)
                }
                it.infrastructureDetail?.let { detail -> Text(detail, style = MaterialTheme.typography.bodySmall) }
                if (it.cleanupConfirmed == false) {
                    Text(stringResource(R.string.autonomous_cleanup_unknown), style = MaterialTheme.typography.bodySmall)
                }
            }
            if (retry.failure != null && retry.sourceSequence == snapshot.sources.core.sequence) {
                Spacer(Modifier.height(8.dp))
                Text(stringResource(R.string.autonomous_retry_rejected), style = MaterialTheme.typography.bodySmall)
            }
            Spacer(Modifier.height(12.dp))
            EnhancedButton(onClick = onRetry, enabled = !running && !retry.pending) {
                Text(stringResource(R.string.retry))
            }
        }
    }
}

@Composable
internal fun AutonomousDiagnosticRows(snapshot: AutonomousDiagnosticSnapshot) {
    val assessment = snapshot.assessment ?: return
    for (id in assessment.required) {
        val status =
            when (id) {
                in assessment.leaked -> R.string.diag_status_leak
                in assessment.proven -> R.string.autonomous_verified
                else -> R.string.dashboard_protection_not_verified
            }
        Row(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
            Text(stringResource(diagnosticVectorLabel(id)), Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
            Text(stringResource(status), style = MaterialTheme.typography.bodySmall)
        }
    }
}

private val diagnosticVectorLabels =
    mapOf(
        "ioctl_flags" to R.string.autonomous_vector_flags,
        "ioctl_mtu" to R.string.autonomous_vector_mtu,
        "getifaddrs" to R.string.autonomous_vector_ifaddrs,
        "netlink_links" to R.string.autonomous_vector_links,
        "ioctl_addresses" to R.string.autonomous_vector_ioctl_addresses,
        "bind_to_device" to R.string.autonomous_vector_bind,
        "netlink_addresses" to R.string.autonomous_vector_addresses,
        "netlink_routes" to R.string.autonomous_vector_routes,
        "netlink_rules" to R.string.autonomous_vector_rules,
        "proc_routes" to R.string.autonomous_vector_proc_routes,
        "proc_interfaces" to R.string.autonomous_vector_proc_interfaces,
        "sys_interfaces" to R.string.autonomous_vector_sys_interfaces,
        "framework_networks" to R.string.autonomous_vector_networks,
        "framework_default_network" to R.string.autonomous_vector_default,
        "framework_capabilities" to R.string.autonomous_vector_capabilities,
        "framework_link_properties" to R.string.autonomous_vector_properties,
        "framework_legacy_info" to R.string.autonomous_vector_legacy,
        "framework_callbacks" to R.string.autonomous_vector_callbacks,
        "framework_pending_intent" to R.string.autonomous_vector_pending_intent,
        "package_visibility" to R.string.autonomous_vector_packages,
        "ports" to R.string.autonomous_vector_ports,
        "filesystem" to R.string.autonomous_vector_filesystem,
    )

private fun diagnosticVectorLabel(id: String): Int = diagnosticVectorLabels[id] ?: R.string.dashboard_protection_not_verified
