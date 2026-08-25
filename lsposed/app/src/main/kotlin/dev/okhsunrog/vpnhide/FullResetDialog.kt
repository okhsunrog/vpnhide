package dev.okhsunrog.vpnhide

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.components.ButtonSpinner
import kotlinx.coroutines.launch

private fun resetBlockerLabel(blocker: ResetBlocker): Int =
    when (blocker) {
        ResetBlocker.KmodInstalled -> R.string.reset_blocker_kmod
        ResetBlocker.KpmInstalled -> R.string.reset_blocker_kpm
        ResetBlocker.ZygiskInstalled -> R.string.reset_blocker_zygisk
        ResetBlocker.PortsInstalled -> R.string.reset_blocker_ports
        ResetBlocker.KernelStillHooked -> R.string.reset_blocker_kernel
        ResetBlocker.LsposedActive -> R.string.reset_blocker_lsposed
    }

private fun uninstallSelf(context: Context) {
    runCatching {
        context.startActivity(
            Intent(Intent.ACTION_DELETE, Uri.parse("package:${context.packageName}"))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }
}

/**
 * Full-reset confirmation flow. Reuses the dashboard's tested module/hook
 * detection ([resetBlockers]) to gate the destructive action: it only deletes
 * leftover service files once every module is removed and the LSPosed hook is
 * inactive (and the kernel is no longer hooked). On success it offers to
 * uninstall the app — it never silently closes.
 */
@Composable
internal fun FullResetDialog(
    selfNeedsRestart: Boolean,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val dash by DashboardCache.state.collectAsState()
    val snap by RootSnapshotCache.snapshot.collectAsState()

    androidx.compose.runtime.LaunchedEffect(Unit) {
        DashboardCache.ensureLoaded(scope, context, selfNeedsRestart)
    }

    var running by remember { mutableStateOf(false) }
    var done by remember { mutableStateOf<Boolean?>(null) } // null = not run; true/false = result

    val blockers =
        dash?.let {
            resetBlockers(
                kmod = it.kmod,
                kpm = it.kpm,
                zygisk = it.zygisk,
                ports = it.ports,
                lsposed = it.lsposed,
                kernelCtlPresent = snap?.sections?.get("proc_exists")?.trim() == "1",
            )
        }
    val ready = blockers != null && blockers.isEmpty()

    AlertDialog(
        onDismissRequest = { if (!running) onDismiss() },
        title = { Text(stringResource(R.string.reset_title)) },
        text = {
            when {
                done == true -> {
                    Text(stringResource(R.string.reset_done_body))
                }

                done == false -> {
                    Text(stringResource(R.string.reset_failed_body))
                }

                running -> {
                    ProgressRow(R.string.reset_running)
                }

                dash == null -> {
                    ProgressRow(R.string.reset_checking)
                }

                ready -> {
                    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        Text(stringResource(R.string.reset_warning))
                        Text(
                            text = stringResource(R.string.reset_instructions),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }

                else -> {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.reset_blocked_intro))
                        blockers?.forEach { blocker ->
                            Text("•  " + stringResource(resetBlockerLabel(blocker)))
                        }
                        Text(
                            text = stringResource(R.string.reset_instructions),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        },
        confirmButton = {
            when {
                done == true -> {
                    TextButton(onClick = { uninstallSelf(context) }) {
                        Text(stringResource(R.string.reset_uninstall))
                    }
                }

                done == false || running -> {
                }

                ready -> {
                    TextButton(
                        onClick = {
                            running = true
                            scope.launch {
                                val (exit, _) = suExecAsync(buildFullResetCommand())
                                running = false
                                done = exit == 0
                                if (exit == 0) {
                                    DashboardCache.refresh(scope, context, selfNeedsRestart)
                                }
                            }
                        },
                    ) {
                        Text(
                            text = stringResource(R.string.reset_confirm),
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }

                else -> {
                }
            }
        },
        dismissButton = {
            if (!running) {
                TextButton(onClick = onDismiss) {
                    Text(stringResource(if (done == true) R.string.reset_close else R.string.btn_cancel))
                }
            }
        },
    )
}

@Composable
private fun ProgressRow(textRes: Int) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
    ) {
        ButtonSpinner(size = 20.dp, color = MaterialTheme.colorScheme.primary)
        Text(stringResource(textRes))
    }
}
