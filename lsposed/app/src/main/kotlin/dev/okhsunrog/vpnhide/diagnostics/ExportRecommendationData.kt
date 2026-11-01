package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.NativeBootOutcome
import dev.okhsunrog.vpnhide.currentObservationValue
import dev.okhsunrog.vpnhide.lsposedHooksActiveThisBoot
import dev.okhsunrog.vpnhide.parseLsposedStateMetadata

internal enum class ExportReason {
    CoreUnavailable,
    Incomplete,
    NoCause,
    KernelHooks,
    KernelCompatibility,
    JavaHooks,
    JavaAttach,
    Configuration,
}

internal data class ExportRecommendation(
    val kernel: Boolean = false,
    val java: Boolean = false,
    val kernelReason: ExportReason = ExportReason.Incomplete,
    val javaReason: ExportReason = ExportReason.Incomplete,
)

/** Null means follow current recommendations. Overrides live only in the open sheet. */
internal data class ExportOverrides(
    val kernel: Boolean? = null,
    val java: Boolean? = null,
) {
    fun resolve(recommendation: ExportRecommendation): Pair<Boolean, Boolean> =
        (kernel ?: recommendation.kernel) to (java ?: recommendation.java)
}

internal fun recommendExport(
    coreAvailable: Boolean,
    report: DiagnosticReport?,
    boot: NativeBootOutcome?,
    javaAttachFailure: Boolean,
): ExportRecommendation {
    if (!coreAvailable) return ExportRecommendation(kernelReason = ExportReason.CoreUnavailable, javaReason = ExportReason.CoreUnavailable)
    val measured = report?.takeIf { it.complete && it.gate == DiagnosticGate.ROUTED }
    val kernelBackend =
        measured?.native?.status is LayerStatus.Active &&
            measured.native.backend in setOf(NativeBackendId.Kmod, NativeBackendId.Builtin, NativeBackendId.Kpm)
    val kernelLeak =
        kernelBackend &&
            measured
                .native
                .checks
                .orEmpty()
                .any { it.owned && it.outcome == CheckOutcome.Leak }
    val javaLeak =
        measured
            ?.java
            ?.takeIf { it.status is LayerStatus.Active }
            ?.checks
            .orEmpty()
            .any { it.owned && it.outcome == CheckOutcome.Leak }
    val kernelCompatibility = kernelCompatibilityEvidence(boot)
    val configuration = configurationOnlyFailure(boot)
    return ExportRecommendation(
        kernel = !configuration && (kernelLeak || kernelCompatibility),
        java = javaLeak || javaAttachFailure,
        kernelReason =
            when {
                configuration -> ExportReason.Configuration
                kernelCompatibility -> ExportReason.KernelCompatibility
                kernelLeak -> ExportReason.KernelHooks
                measured == null -> ExportReason.Incomplete
                else -> ExportReason.NoCause
            },
        javaReason =
            when {
                javaAttachFailure -> ExportReason.JavaAttach
                javaLeak -> ExportReason.JavaHooks
                measured == null -> ExportReason.Incomplete
                else -> ExportReason.NoCause
            },
    )
}

internal fun exportRecommendation(snapshot: DiagnosticSnapshot): ExportRecommendation {
    val root = currentObservationValue(snapshot.sources.root)
    val presentation = snapshot.presentation
    val measurement = presentation.measurement
    val layers = measurement?.context?.coverageLayers
    val report =
        if (exportMeasurementIsCurrent(presentation) && layers != null) {
            buildDiagnosticReport(
                DiagnosticGate.ROUTED,
                presentation.measurementResults,
                layers.backend,
                layers.lsposedActive,
                true,
                layers.installedOptionalHooks,
            )
        } else {
            null
        }
    val raw = root?.sections?.get("lsposed_state").orEmpty()
    val bootId = root?.sections?.get("current_boot_id").orEmpty()
    val meta = parseLsposedStateMetadata(raw)
    val javaFailure =
        lsposedHooksActiveThisBoot(raw, bootId) &&
            (
                !meta["install_failures"].isNullOrBlank() || !meta["broken_fields"].isNullOrBlank() ||
                    meta.values.any { it.contains("matched=0 minApi=0") } || meta["cs_network"].orEmpty().contains("getAllNetworks=0")
            )
    val boot = root?.nativeBoot?.takeIf { it.isFreshFor(bootId) }?.outcome
    return recommendExport(root?.core?.current == true, report, boot, javaFailure)
}

private fun kernelCompatibilityEvidence(boot: NativeBootOutcome?): Boolean =
    when (boot) {
        is NativeBootOutcome.UnsupportedKernel -> {
            boot.backend != NativeBackendId.Zygisk
        }

        is NativeBootOutcome.LoadFailed -> {
            boot.backend != NativeBackendId.Zygisk && !boot.signatureRejected &&
                (
                    boot.kprobes == false || boot.kretprobes == false || boot.stderr.contains("unknown symbol", true) ||
                        boot.stderr.contains("invalid module format", true)
                )
        }

        else -> {
            false
        }
    }

private fun configurationOnlyFailure(boot: NativeBootOutcome?): Boolean =
    boot is NativeBootOutcome.Conflict || boot is NativeBootOutcome.AwaitingAuthentication ||
        boot is NativeBootOutcome.MissingArtifact || boot is NativeBootOutcome.CapacityExceeded

internal fun exportMeasurementIsCurrent(presentation: DiagnosticPresentation): Boolean =
    presentation.applicability == MeasurementApplicability.MatchesLastObservation &&
        presentation.eligibility == DiagnosticEligibility.Eligible && presentation.desiredDeliveryConfirmed &&
        !presentation.probeUnavailable && presentation.measurement?.completed == true &&
        presentation.measurement.interrupted == false && presentation.measurementResults != null
