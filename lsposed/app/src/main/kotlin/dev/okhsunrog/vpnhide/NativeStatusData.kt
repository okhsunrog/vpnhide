package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable
import uniffi.vpnhide_core_ffi.CoreNativeAttemptView
import uniffi.vpnhide_core_ffi.CoreNativeDeliveryView
import uniffi.vpnhide_core_ffi.CoreNativeImplementation
import uniffi.vpnhide_core_ffi.CoreNativeReadModel
import uniffi.vpnhide_core_ffi.CoreNativeRuntimeStatus
import java.util.Locale

/** Core data copied out of mutable transport records; Kotlin does not classify it. */
@Serializable
internal data class NativeImplementationInfo(
    val backend: NativeBackendId,
    val source: String,
    val manifestSha256: String?,
    val sourceCommit: String?,
    val payloadSha256: String?,
)

@Serializable
internal data class NativeRuntimeInfo(
    val backend: NativeBackendId,
    val kernelVersion: Long,
    val installedHooks: Long,
    val statusError: Long,
)

@Serializable
internal data class NativeAttemptInfo(
    val bootId: String,
    val configRevision: Long,
    val operation: Long?,
    val implementation: NativeImplementationInfo?,
    val disposition: String,
)

@Serializable
internal data class NativeDeliveryInfo(
    val backend: NativeBackendId?,
    val operation: Long,
    val configRevision: Long,
    val outcome: String,
    val wireSha256: String?,
)

@Serializable
internal data class NativeStatusInfo(
    val boot: NativeBootStatus?,
    val selected: NativeImplementationInfo?,
    val applied: NativeBackendId?,
    val observed: NativeBackendId?,
    val displayed: NativeImplementationInfo?,
    val observation: String,
    val runtime: NativeRuntimeInfo?,
    val observationError: String?,
    val attempts: List<NativeAttemptInfo>,
    val delivery: NativeDeliveryInfo?,
    val requiredHooks: Long?,
    val installedHooks: Long?,
    val missingHooks: Long?,
    val setup: String,
    val filesystem: String,
    val processCoverageComplete: Boolean,
    val legacyInstalled: List<NativeBackendId>,
)

@Serializable
internal data class NativeStatusSnapshot(
    val current: Boolean,
    val epoch: Long?,
    val sourceSequence: Long,
    val connectionGeneration: Long,
    val configRevision: Long?,
    val model: NativeStatusInfo?,
)

internal fun CoreSessionUpdate.nativeStatusSnapshot(): NativeStatusSnapshot =
    NativeStatusSnapshot(
        current && state?.nativeStatus != null,
        state?.epoch,
        sequence,
        connectionGeneration,
        state?.configRevision,
        state?.nativeStatus,
    )

private fun nativeStatusId(value: Enum<*>): String = value.name.lowercase(Locale.ROOT)

private fun CoreNativeImplementation.toInfo() =
    NativeImplementationInfo(backend.toBackend(), nativeStatusId(source), manifestSha256, sourceCommit, payloadSha256)

private fun CoreNativeRuntimeStatus.toInfo() =
    NativeRuntimeInfo(backend.toBackend(), kernelVersion.toLong(), installedHooks.toLong(), statusError.toLong())

private fun CoreNativeAttemptView.toInfo() =
    NativeAttemptInfo(bootId, configRevision.toLong(), operation?.toLong(), implementation?.toInfo(), nativeStatusId(disposition))

private fun CoreNativeDeliveryView.toInfo() =
    NativeDeliveryInfo(backend?.toBackend(), operation.toLong(), configRevision.toLong(), nativeStatusId(outcome), wireSha256)

internal fun CoreNativeReadModel.toNativeStatusInfo(): NativeStatusInfo =
    NativeStatusInfo(
        boot?.toBootStatus(),
        selected?.toInfo(),
        applied?.toBackend(),
        observed?.toBackend(),
        displayed?.toInfo(),
        nativeStatusId(observation),
        runtime?.toInfo(),
        observationError,
        attempts.map { it.toInfo() },
        delivery?.toInfo(),
        requiredHooks?.toLong(),
        installedHooks?.toLong(),
        missingHooks?.toLong(),
        nativeStatusId(setup),
        nativeStatusId(filesystem),
        processCoverageComplete,
        legacyInstalled.map { it.toBackend() },
    )
