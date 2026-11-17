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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.okhsunrog.vpnhide.R
import dev.okhsunrog.vpnhide.StatusColors
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedCard
import uniffi.vpnhide_core_ffi.CoreAssessmentPhase
import uniffi.vpnhide_core_ffi.CoreCheckOutcome
import uniffi.vpnhide_core_ffi.CoreControlFailure
import uniffi.vpnhide_core_ffi.CoreDiagnosticFailureKind
import uniffi.vpnhide_core_ffi.CoreDiagnosticVerdict
import uniffi.vpnhide_core_ffi.CoreFrameworkControlFailure
import uniffi.vpnhide_core_ffi.CoreFrameworkEvidencePhase
import uniffi.vpnhide_core_ffi.CoreRequiredCheck
import uniffi.vpnhide_core_ffi.CoreReuseBlocker
import uniffi.vpnhide_core_ffi.CoreUnverifiedReason

/** Wording only: every vector/outcome was evaluated by Core. No success is invented. */
@Composable
internal fun AutonomousDiagnosticStatusCard(
    snapshot: AutonomousDiagnosticSnapshot,
    onRetry: () -> Unit,
) {
    val assessment = snapshot.assessment
    val retry by DiagnosticsCache.retryState.collectAsStateWithLifecycle()
    val running = snapshot.current && assessment?.phase == CoreAssessmentPhase.RUNNING
    val hasLeaks = assessment?.verdict == CoreDiagnosticVerdict.LEAKED
    val allVerified = assessment?.phase == CoreAssessmentPhase.FINISHED && assessment.verdict == CoreDiagnosticVerdict.PROVEN
    val title =
        when {
            snapshot.availability == AutonomousAvailability.Initializing || running -> R.string.autonomous_running
            !snapshot.current -> R.string.autonomous_unavailable
            hasLeaks -> R.string.autonomous_leaks
            assessment?.phase == CoreAssessmentPhase.INFRASTRUCTURE -> R.string.autonomous_unavailable
            allVerified -> R.string.autonomous_all_verified
            else -> R.string.autonomous_incomplete
        }
    val color =
        when {
            snapshot.current && hasLeaks -> StatusColors.errorContainer()
            snapshot.current && allVerified -> StatusColors.successContainer()
            snapshot.current && assessment?.phase == CoreAssessmentPhase.FINISHED -> StatusColors.warningContainer()
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
                Text(
                    stringResource(
                        R.string.autonomous_tally,
                        it.checks.count { row -> row.outcome == CoreCheckOutcome.Proven },
                        it.checks.count { row -> row.outcome == CoreCheckOutcome.Leaked },
                        it.checks.count { row -> row.outcome is CoreCheckOutcome.Unverified },
                    ),
                )
                if (it.reuseBlockers.contains(CoreReuseBlocker.LOADED_GENERATIONS_INCOMPLETE)) {
                    Text(stringResource(R.string.autonomous_process_coverage), style = MaterialTheme.typography.bodySmall)
                }
                it.failure?.let { failure -> Text(failure.detail, style = MaterialTheme.typography.bodySmall) }
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
    for (row in assessment.checks) {
        val androidDenied = row.androidAccessDenied
        val outcome = row.outcome
        val status =
            when (outcome) {
                CoreCheckOutcome.Leaked -> {
                    R.string.diag_status_leak
                }

                CoreCheckOutcome.Proven -> {
                    if (androidDenied) R.string.autonomous_kernel_verified else R.string.autonomous_verified
                }

                is CoreCheckOutcome.Unverified -> {
                    if (outcome.reason == CoreUnverifiedReason.AndroidAccessDenied) {
                        R.string.autonomous_blocked_by_android
                    } else {
                        R.string.dashboard_protection_not_verified
                    }
                }
            }
        val detail =
            buildList {
                add(stringResource(status))
                if (androidDenied && status != R.string.autonomous_blocked_by_android) {
                    add(stringResource(R.string.autonomous_blocked_by_android))
                }
                if (outcome is CoreCheckOutcome.Unverified && outcome.reason != CoreUnverifiedReason.AndroidAccessDenied) {
                    addAll(diagnosticReasons(outcome.reason))
                }
            }
        Row(Modifier.fillMaxWidth().padding(vertical = 8.dp)) {
            Text(stringResource(diagnosticVectorLabel(row.check)), Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
            Text(detail.joinToString("\n"), textAlign = TextAlign.End, style = MaterialTheme.typography.bodySmall)
        }
    }
}

@Composable
private fun diagnosticReasons(reason: CoreUnverifiedReason): List<String> =
    when (reason) {
        CoreUnverifiedReason.AwaitingEvidence -> {
            emptyList()
        }

        CoreUnverifiedReason.EvidenceUnavailable -> {
            listOf(stringResource(R.string.autonomous_reason_no_evidence))
        }

        CoreUnverifiedReason.AndroidAccessDenied -> {
            listOf(stringResource(R.string.autonomous_blocked_by_android))
        }

        is CoreUnverifiedReason.NativeControls -> {
            reason.controls.map { stringResource(nativeReasonLabel(it)) }.distinct()
        }

        is CoreUnverifiedReason.FrameworkControls -> {
            reason.failures
                .map {
                    val message = stringResource(frameworkReasonLabel(it.reason))
                    val phase = it.phase
                    if (phase ==
                        null
                    ) {
                        message
                    } else {
                        stringResource(R.string.autonomous_reason_in_phase, message, stringResource(phaseLabel(phase)))
                    }
                }.distinct()
        }

        is CoreUnverifiedReason.Interrupted -> {
            listOf(stringResource(interruptionLabel(reason.failure)))
        }
    }

private fun nativeReasonLabel(reason: CoreControlFailure): Int =
    when (reason) {
        CoreControlFailure.INVALID_SPEC -> R.string.autonomous_reason_invalid

        CoreControlFailure.MISSING_POSITIVE_CONTROL -> R.string.autonomous_reason_missing_positive

        CoreControlFailure.MISSING_NEGATIVE_CONTROL -> R.string.autonomous_reason_missing_negative

        CoreControlFailure.FIXTURE_CHANGED, CoreControlFailure.ORDINARY_OBJECTS_CHANGED -> R.string.autonomous_reason_changed

        CoreControlFailure.PROBE_UNAVAILABLE -> R.string.autonomous_reason_probe_unavailable

        CoreControlFailure.CLEANUP_UNCONFIRMED -> R.string.autonomous_cleanup_unknown

        CoreControlFailure.NETLINK_LINK_REQUEST_DENIED,
        CoreControlFailure.PROC_ROUTE_REQUEST_DENIED,
        CoreControlFailure.PROC_INTERFACE_REQUEST_DENIED,
        -> R.string.autonomous_blocked_by_android
    }

private fun frameworkReasonLabel(reason: CoreFrameworkControlFailure): Int =
    when (reason) {
        CoreFrameworkControlFailure.INVALID_ENVELOPE -> R.string.autonomous_reason_invalid

        CoreFrameworkControlFailure.IDENTITY_CHANGED,
        CoreFrameworkControlFailure.SOURCE_CHANGED,
        CoreFrameworkControlFailure.ORDINARY_OBJECTS_CHANGED,
        -> R.string.autonomous_reason_changed

        CoreFrameworkControlFailure.MISSING_POSITIVE_CONTROL -> R.string.autonomous_reason_missing_positive

        CoreFrameworkControlFailure.MISSING_NEGATIVE_CONTROL -> R.string.autonomous_reason_missing_negative

        CoreFrameworkControlFailure.PRODUCTION_PATH_NOT_OBSERVED -> R.string.autonomous_reason_hook_not_observed

        CoreFrameworkControlFailure.PROBE_UNAVAILABLE -> R.string.autonomous_reason_probe_unavailable

        CoreFrameworkControlFailure.CLEANUP_UNCONFIRMED -> R.string.autonomous_cleanup_unknown
    }

private fun phaseLabel(phase: CoreFrameworkEvidencePhase): Int =
    when (phase) {
        CoreFrameworkEvidencePhase.PREPARED -> R.string.autonomous_phase_prepared
        CoreFrameworkEvidencePhase.RAW -> R.string.autonomous_phase_raw
        CoreFrameworkEvidencePhase.PROTECTED -> R.string.autonomous_phase_protected
        CoreFrameworkEvidencePhase.REVOKED -> R.string.autonomous_phase_revoked
    }

private fun interruptionLabel(failure: CoreDiagnosticFailureKind): Int =
    when (failure) {
        CoreDiagnosticFailureKind.TIMEOUT -> R.string.autonomous_reason_timeout
        CoreDiagnosticFailureKind.BUSY -> R.string.autonomous_reason_busy
        CoreDiagnosticFailureKind.CANCELLED -> R.string.autonomous_reason_cancelled
        CoreDiagnosticFailureKind.UNAVAILABLE -> R.string.autonomous_reason_probe_unavailable
        CoreDiagnosticFailureKind.CLEANUP_UNCONFIRMED -> R.string.autonomous_cleanup_unknown
    }

@Suppress("CyclomaticComplexMethod") // The exhaustive table requires a label for every check added to Core.
private fun diagnosticVectorLabel(check: CoreRequiredCheck): Int =
    when (check) {
        CoreRequiredCheck.IOCTL_FLAGS -> R.string.autonomous_vector_flags
        CoreRequiredCheck.IOCTL_MTU -> R.string.autonomous_vector_mtu
        CoreRequiredCheck.GETIFADDRS -> R.string.autonomous_vector_ifaddrs
        CoreRequiredCheck.NETLINK_LINKS -> R.string.autonomous_vector_links
        CoreRequiredCheck.IOCTL_ADDRESSES -> R.string.autonomous_vector_ioctl_addresses
        CoreRequiredCheck.BIND_TO_DEVICE -> R.string.autonomous_vector_bind
        CoreRequiredCheck.NETLINK_ADDRESSES -> R.string.autonomous_vector_addresses
        CoreRequiredCheck.NETLINK_ROUTES -> R.string.autonomous_vector_routes
        CoreRequiredCheck.NETLINK_RULES -> R.string.autonomous_vector_rules
        CoreRequiredCheck.PROC_ROUTES -> R.string.autonomous_vector_proc_routes
        CoreRequiredCheck.PROC_INTERFACES -> R.string.autonomous_vector_proc_interfaces
        CoreRequiredCheck.SYS_INTERFACES -> R.string.autonomous_vector_sys_interfaces
        CoreRequiredCheck.FRAMEWORK_NETWORKS -> R.string.autonomous_vector_networks
        CoreRequiredCheck.FRAMEWORK_DEFAULT_NETWORK -> R.string.autonomous_vector_default
        CoreRequiredCheck.FRAMEWORK_CAPABILITIES -> R.string.autonomous_vector_capabilities
        CoreRequiredCheck.FRAMEWORK_LINK_PROPERTIES -> R.string.autonomous_vector_properties
        CoreRequiredCheck.FRAMEWORK_LEGACY_INFO -> R.string.autonomous_vector_legacy
        CoreRequiredCheck.FRAMEWORK_CALLBACKS -> R.string.autonomous_vector_callbacks
        CoreRequiredCheck.FRAMEWORK_PENDING_INTENT -> R.string.autonomous_vector_pending_intent
        CoreRequiredCheck.PACKAGE_VISIBILITY -> R.string.autonomous_vector_packages
        CoreRequiredCheck.PORTS -> R.string.autonomous_vector_ports
        CoreRequiredCheck.FILESYSTEM -> R.string.autonomous_vector_filesystem
    }
