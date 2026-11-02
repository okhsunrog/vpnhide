package dev.okhsunrog.vpnhide

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard

/** Wording only. Setup readiness never replaces Core's independent diagnostic result. */
@Composable
internal fun NativeCoreStatusCard(snapshot: NativeStatusSnapshot) {
    val model = snapshot.model
    val backend = model?.displayed?.backend
    val current = snapshot.current
    val status = if (current) model?.setup else null
    val color =
        when (status) {
            "runtime_conflict", "delivery_failed", "backend_error" -> StatusColors.errorContainer()
            "partial_hooks" -> StatusColors.warningContainer()
            else -> StatusColors.neutralContainer()
        }
    EnhancedCard(modifier = Modifier.fillMaxWidth(), color = color) {
        Column(Modifier.padding(16.dp)) {
            Text(
                stringResource(R.string.dashboard_native_backend),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
            )
            if (backend != null) Text(stringResource(nativeBackendNameResource(backend)), style = MaterialTheme.typography.bodyLarge)
            Text(stringResource(nativeSetupResource(status)), style = MaterialTheme.typography.bodyMedium)
            if (current && model?.displayed?.source in setOf("bundled", "builtin", "legacy")) {
                Spacer(Modifier.height(4.dp))
                Text(stringResource(nativeSourceResource(model?.displayed?.source)), style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

private fun nativeBackendNameResource(id: NativeBackendId): Int =
    when (id) {
        NativeBackendId.Kmod -> R.string.dashboard_backend_kmod
        NativeBackendId.Builtin -> R.string.dashboard_backend_builtin
        NativeBackendId.Kpm -> R.string.dashboard_backend_kpm
        NativeBackendId.Zygisk -> R.string.dashboard_backend_zygisk
    }

private val nativeSetupResources =
    mapOf(
        "unavailable" to R.string.native_status_unavailable,
        "no_selection" to R.string.native_status_no_selection,
        "awaiting_runtime" to R.string.native_status_awaiting_runtime,
        "runtime_conflict" to R.string.native_status_conflict,
        "delivery_pending" to R.string.native_status_pending,
        "delivery_failed" to R.string.native_status_failed,
        "delivery_unknown" to R.string.native_status_unknown,
        "policy_unknown" to R.string.native_status_unknown,
        "partial_hooks" to R.string.native_status_partial,
        "backend_error" to R.string.native_status_failed,
        "setup_ready" to R.string.native_status_ready,
        "awaiting_process_evidence" to R.string.native_status_processes,
    )

private fun nativeSetupResource(id: String?): Int = nativeSetupResources[id] ?: R.string.native_status_unavailable

private fun nativeSourceResource(id: String?): Int =
    when (id) {
        "bundled" -> R.string.native_status_bundled
        "builtin" -> R.string.native_status_builtin
        else -> R.string.native_status_legacy
    }
