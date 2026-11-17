package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import uniffi.vpnhide_core_ffi.CoreAddressCheck
import uniffi.vpnhide_core_ffi.CoreCheckOutcome
import uniffi.vpnhide_core_ffi.CoreDiagnosticAssessment
import uniffi.vpnhide_core_ffi.CoreDiagnosticRun
import uniffi.vpnhide_core_ffi.CoreFilesystemCheck
import uniffi.vpnhide_core_ffi.CoreFrameworkAttribution
import uniffi.vpnhide_core_ffi.CoreFrameworkFailureScope
import uniffi.vpnhide_core_ffi.CoreProcInterfaceCheck
import uniffi.vpnhide_core_ffi.CoreRouteCheck
import uniffi.vpnhide_core_ffi.CoreSysInterfaceCheck
import uniffi.vpnhide_core_ffi.CoreUnverifiedReason
import uniffi.vpnhide_core_ffi.nativeRawReceiptJson
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
    val binding: AutonomousBindInfo? = null,
    val addresses: AutonomousNativeReadInfo? = null,
    val procInterfaces: AutonomousNativeReadInfo? = null,
    val nativeRaw: JsonElement? = null,
    val sysInterfaces: AutonomousNativeReadInfo? = null,
    val filesystem: AutonomousNativeReadInfo? = null,
    val routes: AutonomousNativeReadInfo? = null,
)

@Serializable
internal data class AutonomousNativeReadInfo(
    val provenChecks: List<String>,
    val leakedChecks: List<String>,
    val controls: List<String>,
)

private fun CoreAddressCheck.infoName(): String = "${actor.name.lowercase(Locale.ROOT)}.${vector.name.lowercase(Locale.ROOT)}"

private fun CoreProcInterfaceCheck.infoName(): String = "${actor.name.lowercase(Locale.ROOT)}.${surface.name.lowercase(Locale.ROOT)}"

private fun CoreSysInterfaceCheck.infoName(): String = "${actor.name.lowercase(Locale.ROOT)}.${surface.name.lowercase(Locale.ROOT)}"

private fun CoreRouteCheck.infoName(): String = "${actor.name.lowercase(Locale.ROOT)}.${surface.name.lowercase(Locale.ROOT)}"

private fun CoreFilesystemCheck.infoName(): String =
    "${actor.name.lowercase(Locale.ROOT)}.${surface.name.lowercase(Locale.ROOT)}.${vector.name.lowercase(Locale.ROOT)}"

@Serializable
internal data class AutonomousBindInfo(
    val provenActors: List<String>,
    val leakedActors: List<String>,
    val controls: List<String>,
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
    val verdict: String,
    val checks: List<AutonomousCheckInfo>,
    val nativeControls: List<String>,
    val frameworkControls: List<String>,
    val reuseBlockers: List<String>,
    val failure: AutonomousFailureInfo?,
    val cleanupConfirmed: Boolean?,
)

@Serializable
internal data class AutonomousCheckInfo(
    val check: String,
    val outcome: String,
    val androidAccessDenied: Boolean,
    val reason: JsonElement? = null,
)

@Serializable
internal data class AutonomousFailureInfo(
    val stage: String,
    val kind: String,
    val detail: String,
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
        binding?.let { result ->
            AutonomousBindInfo(
                result.proven.map { it.name.lowercase(Locale.ROOT) },
                result.leaked.map { it.name.lowercase(Locale.ROOT) },
                result.controls.map { it.name.lowercase(Locale.ROOT) },
            )
        },
        addresses?.let { result ->
            AutonomousNativeReadInfo(
                result.proven.map { it.infoName() },
                result.leaked.map { it.infoName() },
                result.controls.map { it.name.lowercase(Locale.ROOT) },
            )
        },
        procInterfaces?.let { result ->
            AutonomousNativeReadInfo(
                result.proven.map { it.infoName() },
                result.leaked.map { it.infoName() },
                result.controls.map { it.name.lowercase(Locale.ROOT) },
            )
        },
        nativeRaw?.let { Json.parseToJsonElement(nativeRawReceiptJson(it)) },
        sysInterfaces?.let { result ->
            AutonomousNativeReadInfo(
                result.proven.map { it.infoName() },
                result.leaked.map { it.infoName() },
                result.controls.map { it.name.lowercase(Locale.ROOT) },
            )
        },
        filesystem?.let { result ->
            AutonomousNativeReadInfo(
                result.proven.map { it.infoName() },
                result.leaked.map { it.infoName() },
                result.controls.map { it.name.lowercase(Locale.ROOT) },
            )
        },
        routes?.let { result ->
            AutonomousNativeReadInfo(
                result.proven.map { it.infoName() },
                result.leaked.map { it.infoName() },
                result.controls.map { it.name.lowercase(Locale.ROOT) },
            )
        },
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
        verdict.name.lowercase(Locale.ROOT),
        checks.map { row ->
            val outcome = row.outcome
            val name =
                when (outcome) {
                    CoreCheckOutcome.Proven -> "proven"
                    CoreCheckOutcome.Leaked -> "leaked"
                    is CoreCheckOutcome.Unverified -> "unverified"
                }
            AutonomousCheckInfo(
                row.check.name.lowercase(Locale.ROOT),
                name,
                row.androidAccessDenied,
                (outcome as? CoreCheckOutcome.Unverified)?.reason?.toDiagnosticInfo(),
            )
        },
        nativeControls.map { it.name.lowercase(Locale.ROOT) },
        frameworkControls.map { it.name.lowercase(Locale.ROOT) },
        reuseBlockers.map { it.name.lowercase(Locale.ROOT) },
        failure?.let { AutonomousFailureInfo(it.stage.name.lowercase(Locale.ROOT), it.kind.name.lowercase(Locale.ROOT), it.detail) },
        cleanupConfirmed,
    )

private fun CoreUnverifiedReason.toDiagnosticInfo(): JsonElement =
    buildJsonObject {
        when (val reason = this@toDiagnosticInfo) {
            CoreUnverifiedReason.AwaitingEvidence -> {
                put("kind", "awaiting_evidence")
            }

            CoreUnverifiedReason.EvidenceUnavailable -> {
                put("kind", "evidence_unavailable")
            }

            CoreUnverifiedReason.AndroidAccessDenied -> {
                put("kind", "android_access_denied")
            }

            is CoreUnverifiedReason.NativeControls -> {
                put("kind", "native_controls")
                putJsonArray("controls") { reason.controls.forEach { add(it.name.lowercase(Locale.ROOT)) } }
            }

            is CoreUnverifiedReason.FrameworkControls -> {
                put("kind", "framework_controls")
                putJsonArray("failures") {
                    for (failure in reason.failures) {
                        add(
                            buildJsonObject {
                                put("reason", failure.reason.name.lowercase(Locale.ROOT))
                                failure.phase?.let { put("phase", it.name.lowercase(Locale.ROOT)) }
                                when (val scope = failure.scope) {
                                    CoreFrameworkFailureScope.Run -> {
                                        put("scope", "run")
                                    }

                                    CoreFrameworkFailureScope.NetworkView -> {
                                        put("scope", "network_view")
                                    }

                                    is CoreFrameworkFailureScope.Check -> {
                                        put("scope", "check")
                                        put("check", scope.check.name.lowercase(Locale.ROOT))
                                    }
                                }
                            },
                        )
                    }
                }
            }

            is CoreUnverifiedReason.Interrupted -> {
                put("kind", "interrupted")
                put("stage", reason.stage.name.lowercase(Locale.ROOT))
                put("failure", reason.failure.name.lowercase(Locale.ROOT))
            }
        }
    }

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
        assessment = assessment?.toDiagnosticInfo(),
    )
