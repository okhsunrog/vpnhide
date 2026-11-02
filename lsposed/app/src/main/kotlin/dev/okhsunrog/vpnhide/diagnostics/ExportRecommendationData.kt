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

/** Recommendations consume the same accepted Core facts as UI/export/agent. */
internal fun exportRecommendation(snapshot: AutonomousDiagnosticSnapshot): ExportRecommendation {
    val core = snapshot.sources.core
    if (!core.current) return ExportRecommendation(kernelReason = ExportReason.CoreUnavailable, javaReason = ExportReason.CoreUnavailable)
    val boot = core.state?.nativeBoot?.outcome
    val compatibility = kernelCompatibilityEvidence(boot)
    val configuration = configurationOnlyFailure(boot)
    val leaks =
        snapshot.assessment
            ?.leaked
            .orEmpty()
            .takeIf { snapshot.current }
            .orEmpty()
    val native = leaks.any { !it.startsWith("framework_") && it !in setOf("package_visibility", "ports") }
    val java = leaks.any { it.startsWith("framework_") || it == "package_visibility" }
    val kernel = native && core.state?.backend?.selected in setOf(NativeBackendId.Kmod, NativeBackendId.Builtin, NativeBackendId.Kpm)
    return ExportRecommendation(
        kernel = !configuration && (kernel || compatibility),
        java = java,
        kernelReason =
            when {
                configuration -> ExportReason.Configuration
                compatibility -> ExportReason.KernelCompatibility
                kernel -> ExportReason.KernelHooks
                else -> ExportReason.Incomplete
            },
        javaReason = if (java) ExportReason.JavaHooks else ExportReason.Incomplete,
    )
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
