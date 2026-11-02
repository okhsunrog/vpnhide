package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.serialization.Serializable
import uniffi.vpnhide_core_ffi.CoreDiagnosticAssessment
import uniffi.vpnhide_core_ffi.CoreDiagnosticRun
import uniffi.vpnhide_core_ffi.CoreFrameworkAttribution
import java.util.Locale

/** Exported evidence is retained Core publication data, not a post-baseline suite. */
@Serializable
internal data class AutonomousDiagnosticInfo(
    val availability: String,
    val current: Boolean,
    val sourceEpoch: Long?,
    val sourceConfigRevision: Long?,
    val sourceSequence: Long,
    val connectionGeneration: Long,
    val snapshotRevision: Long,
    val rootObservationId: Long?,
    val rootGeneration: Long?,
    val rootJoined: Boolean,
    val rootSourceEpoch: Long?,
    val rootSourceSequence: Long?,
    val rootSourceConnectionGeneration: Long?,
    val rootSourceConfigRevision: Long?,
    val selectedBackend: String?,
    val appliedBackend: String?,
    val mutationHeld: Boolean,
    val heldResources: List<String>,
    val provenance: String = "retained_core_publication",
    val run: AutonomousRunInfo?,
    val assessment: AutonomousAssessmentInfo?,
)

@Serializable
internal data class AutonomousRunInfo(
    val session: Long,
    val coreEpoch: Long,
    val bootId: String,
    val policySha256: String,
    val runtimeSha256: String,
    val platformSha256: String?,
    val loadedGenerationsSha256: String?,
    val loadedGenerationsComplete: Boolean,
    val contract: Int,
    val stale: Boolean,
    val frameworkAttribution: AutonomousFrameworkAttributionInfo?,
)

@Serializable
internal data class AutonomousFrameworkAttributionInfo(
    val session: Long,
    val coreEpoch: Long,
    val bootId: String,
    val workerUid: Long,
    val workerPid: Long,
    val workerStartTicks: Long,
    val systemServerPid: Long,
    val systemServerStartTicks: Long,
    val hookSourceSha256: String,
    val sourceProfileSha256: String,
)

@Serializable
internal data class AutonomousAssessmentInfo(
    val phase: String,
    val required: List<String>,
    val proven: List<String>,
    val leaked: List<String>,
    val unverified: List<String>,
    val nativeControls: List<String>,
    val frameworkControls: List<String>,
    val reuseBlockers: List<String>,
    val infrastructureDetail: String?,
    val cleanupConfirmed: Boolean?,
)

internal fun CoreDiagnosticRun.toDiagnosticInfo(): AutonomousRunInfo =
    AutonomousRunInfo(
        session.toLong(),
        coreEpoch.toLong(),
        bootId,
        policySha256,
        runtimeSha256,
        platformSha256,
        loadedGenerationsSha256,
        loadedGenerationsComplete,
        contract.toInt(),
        stale,
        framework?.attribution?.toDiagnosticInfo(),
    )

private fun CoreFrameworkAttribution.toDiagnosticInfo(): AutonomousFrameworkAttributionInfo =
    AutonomousFrameworkAttributionInfo(
        session.toLong(),
        coreEpoch.toLong(),
        bootId,
        workerUid.toLong(),
        workerPid.toLong(),
        workerStartTicks.toLong(),
        systemServerPid.toLong(),
        systemServerStartTicks.toLong(),
        hookSourceSha256,
        sourceProfileSha256,
    )

/** Only vocabulary conversion; Core has already evaluated every outcome. */
internal fun CoreDiagnosticAssessment.toDiagnosticInfo(): AutonomousAssessmentInfo =
    AutonomousAssessmentInfo(
        phase.name.lowercase(Locale.ROOT),
        required.map { it.name.lowercase(Locale.ROOT) },
        proven.map { it.name.lowercase(Locale.ROOT) },
        leaked.map { it.name.lowercase(Locale.ROOT) },
        unverified.map { it.name.lowercase(Locale.ROOT) },
        nativeControls.map { it.name.lowercase(Locale.ROOT) },
        frameworkControls.map { it.name.lowercase(Locale.ROOT) },
        reuseBlockers.map { it.name.lowercase(Locale.ROOT) },
        infrastructureDetail,
        cleanupConfirmed,
    )

internal fun AutonomousDiagnosticSnapshot.toDiagnosticInfo(): AutonomousDiagnosticInfo =
    AutonomousDiagnosticInfo(
        availability.name.replace(Regex("([a-z])([A-Z])"), "$1_$2").lowercase(Locale.ROOT),
        current,
        sources.core.state?.epoch,
        sources.core.state?.configRevision,
        sources.core.sequence,
        sources.core.connectionGeneration,
        revision,
        rootSnapshot?.observationId,
        rootSnapshot?.generation,
        rootJoined,
        rootSnapshot?.core?.state?.epoch,
        rootSnapshot?.core?.sequence,
        rootSnapshot?.core?.connectionGeneration,
        rootSnapshot?.core?.state?.configRevision,
        sources.core.state
            ?.backend
            ?.selected
            ?.name
            ?.lowercase(Locale.ROOT),
        sources.core.state
            ?.backend
            ?.applied
            ?.name
            ?.lowercase(Locale.ROOT),
        sources.core.state?.mutationHeld ?: false,
        sources.core.state
            ?.heldResources
            ?.map { it.name.lowercase(Locale.ROOT) }
            .orEmpty(),
        run = run,
        assessment = assessment,
    )
