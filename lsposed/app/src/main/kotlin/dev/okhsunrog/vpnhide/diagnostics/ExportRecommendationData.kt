package dev.okhsunrog.vpnhide.diagnostics

import uniffi.vpnhide_core_ffi.recommendDiagnosticAttachments

internal typealias ExportReason = uniffi.vpnhide_core_ffi.CoreAttachmentReason
internal typealias ExportRecommendation = uniffi.vpnhide_core_ffi.CoreAttachmentRecommendation

/** Null means follow current recommendations. Overrides live only in the open sheet. */
internal data class ExportOverrides(
    val kernel: Boolean? = null,
    val java: Boolean? = null,
) {
    fun resolve(recommendation: ExportRecommendation): Pair<Boolean, Boolean> =
        (kernel ?: recommendation.kernel) to (java ?: recommendation.java)
}

/** Only the accepted observation/config join is qualified by this Android adapter. */
internal fun exportRecommendation(snapshot: AutonomousDiagnosticSnapshot): ExportRecommendation =
    recommendDiagnosticAttachments(
        snapshot.sources.core.state
            ?.attachmentEvidence,
        snapshot.sources.core.current,
        snapshot.current,
    )
