package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.MeasurementCoverage
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.reportGate

/** What the Dashboard tiles show, with the report they were derived from (null unless measured). */
internal data class ProtectionVerdict(
    val check: ProtectionCheck,
    val report: DiagnosticReport?,
)

/**
 * The tiles' verdict from the shared diagnostic presentation. The latest complete
 * measurement is rendered against the layers it was measured with (§6), never the
 * ones the Dashboard sees now; a later attempt that did not complete leaves it in
 * place and is named by the hero note instead (T11). Without a measurement the
 * latest attempt's blocking eligibility becomes the gate, and anything else —
 * an execution failure, an interrupted run, a condition with no gate vocabulary,
 * no attempt at all — is Failed: could not measure, as distinct from VPN off.
 * [liveLayers] only attributes a measurement that carries no layers of its own.
 */
internal fun protectionVerdict(
    presentation: DiagnosticPresentation,
    liveLayers: MeasurementCoverage,
): ProtectionVerdict =
    when (val gate = presentation.reportGate()) {
        DiagnosticGate.ROUTED -> {
            val layers = presentation.measurement?.context?.coverageLayers ?: liveLayers
            val report =
                buildDiagnosticReport(
                    gate = DiagnosticGate.ROUTED,
                    results = presentation.measurementResults,
                    backend = layers.backend,
                    lsposedActive = layers.lsposedActive,
                    complete = true,
                    installedOptionalHooks = layers.installedOptionalHooks,
                )
            ProtectionVerdict(ProtectionCheck.Checked(report.native.status, report.java.status), report)
        }

        null -> {
            ProtectionVerdict(ProtectionCheck.Failed, report = null)
        }

        else -> {
            ProtectionVerdict(ProtectionCheck.Blocked(gate), report = null)
        }
    }
