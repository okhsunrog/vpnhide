package dev.okhsunrog.vpnhide.maintenance

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Card
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.okhsunrog.vpnhide.CoreSessions
import dev.okhsunrog.vpnhide.R
import uniffi.vpnhide_core_ffi.CoreMaintenanceStage

@Composable
internal fun CoreMaintenanceCard() {
    val context = LocalContext.current
    val update by CoreSessions.updates.collectAsStateWithLifecycle()
    val busy by CoreMaintenance.busy.collectAsStateWithLifecycle()
    val issue by CoreMaintenance.issue.collectAsStateWithLifecycle()
    val pending by CoreMaintenance.pending.collectAsStateWithLifecycle()
    val available = remember(context) { CoreMaintenance.targetAvailable(context) }
    val status = update.state?.maintenance
    val active = busy || status?.stage?.isRunning() == true
    val picker =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
            uri?.let { CoreMaintenance.stage(context, it) }
        }
    if (!available && status == null && !issue) return
    Card {
        Column {
            Text(stringResource(R.string.core_maintenance_title))
            if (!update.current && status != null) Text(stringResource(R.string.core_maintenance_unknown))
            when (status?.stage?.takeIf { update.current }) {
                CoreMaintenanceStage.PENDING_RESTART -> Text(stringResource(R.string.core_maintenance_pending_restart))
                CoreMaintenanceStage.VERIFIED -> Text(stringResource(R.string.core_maintenance_verified))
                CoreMaintenanceStage.FAILED_KNOWN -> Text(stringResource(R.string.core_maintenance_failed))
                CoreMaintenanceStage.UNKNOWN -> Text(stringResource(R.string.core_maintenance_unknown))
                else -> Unit
            }
            if (active) LinearProgressIndicator()
            if (issue) {
                Text(stringResource(R.string.core_maintenance_unknown))
                if (pending !=
                    null
                ) {
                    TextButton(onClick = CoreMaintenance::refresh) { Text(stringResource(R.string.core_maintenance_refresh)) }
                }
            }
            if (available && update.current && !active && pending == null &&
                (status == null || status.stage in setOf(CoreMaintenanceStage.VERIFIED, CoreMaintenanceStage.FAILED_KNOWN))
            ) {
                TextButton(onClick = { picker.launch(arrayOf("application/zip", "application/octet-stream")) }) {
                    Text(stringResource(R.string.core_maintenance_choose))
                }
            }
        }
    }
}
