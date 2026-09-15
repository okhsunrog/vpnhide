package dev.okhsunrog.vpnhide.debug

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CanonicalMutation
import dev.okhsunrog.vpnhide.CanonicalWriteResult
import dev.okhsunrog.vpnhide.CaptureLoggingEvent
import dev.okhsunrog.vpnhide.OperationSource
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.parseCanonicalConfig
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicLong

private val nextDebugCaptureSessionId = AtomicLong(1L)

internal data class DebugCaptureLoggingSession(
    val id: Long,
    val originalDebug: Boolean,
    val apply: DebugCaptureLoggingStep,
    val restore: DebugCaptureLoggingStep? = null,
) {
    val forced: Boolean
        get() = !originalDebug

    fun withRestore(restore: DebugCaptureLoggingStep?): DebugCaptureLoggingSession = copy(restore = restore)

    fun toDebugCaptureInfo(): DebugCaptureInfo =
        DebugCaptureInfo(
            forced = forced,
            applyExit = apply.commandExit,
            restoreExit = restore?.commandExit,
            detail = apply.detail.ifBlank { null },
        )

    fun toText(): String =
        buildString {
            appendLine("sessionId=$id")
            appendLine("originalDebug=$originalDebug")
            appendLine("forced=$forced")
            appendLine()
            appendLine("=== apply ===")
            append(apply.toText())
            appendLine()
            appendLine("=== restore ===")
            append(restore?.toText() ?: "(not needed)")
            appendLine()
            appendLine("=== zygisk note ===")
            appendLine(
                "Zygisk reads the native config when a target process specializes. " +
                    "This capture can enable future Zygisk logs, but already-running target processes keep their old in-memory config.",
            )
            appendLine(
                "Useful Zygisk debug lines include target UID selection, selected hook mask, hook-install failures, " +
                    "successful selected-libc hook install, heartbeat writes, and anon-region scrub traces.",
            )
        }.trimEnd()
}

internal data class DebugCaptureLoggingStep(
    val requestedEnabled: Boolean,
    val source: String,
    val rootSnapshotExit: Int?,
    val commandExit: Int?,
    val detail: String,
) {
    fun toText(): String =
        buildString {
            appendLine("requestedEnabled=$requestedEnabled")
            appendLine("source=$source")
            appendLine("rootSnapshotExit=${rootSnapshotExit?.toString() ?: "(n/a)"}")
            appendLine("commandExit=${commandExit?.toString() ?: "(n/a)"}")
            appendLine("detail=${detail.ifBlank { "(empty)" }}")
        }.trimEnd()
}

internal suspend fun beginDebugCaptureLogging(): DebugCaptureLoggingSession =
    withContext(NonCancellable) {
        val id = nextDebugCaptureSessionId.getAndIncrement()
        val original =
            CanonicalConfigRepository.state.value.confirmed
                ?.debug ?: false
        val result =
            CanonicalConfigRepository.commit(
                CanonicalMutation(emptyList(), source = OperationSource.System, captureEvent = CaptureLoggingEvent.Acquire(id)),
            )
        DebugCaptureLoggingSession(id, original, captureStep(true, result))
    }

internal suspend fun restoreDebugCaptureLogging(session: DebugCaptureLoggingSession): DebugCaptureLoggingStep =
    withContext(NonCancellable) {
        val result =
            CanonicalConfigRepository.commit(
                CanonicalMutation(emptyList(), source = OperationSource.System, captureEvent = CaptureLoggingEvent.Release(session.id)),
            )
        captureStep(
            (
                CanonicalConfigRepository.state.value.confirmed
                    ?.debugSwitch ?: session.originalDebug
            ) ||
                CanonicalConfigRepository.state.value.activeCaptures > 0,
            result,
        )
    }

private fun captureStep(
    enabled: Boolean,
    result: CanonicalWriteResult,
) = DebugCaptureLoggingStep(
    requestedEnabled = enabled,
    source = "config_coordinator",
    rootSnapshotExit = null,
    commandExit = result.exitCode,
    detail = result.operation?.failure?.name ?: result.output,
)

/**
 * Parse canonical config from snapshot and clone it with only [`debug`] changed.
 * Callers are responsible for having canonical JSON available as the source of
 * truth.
 */
internal fun debugToggledCanonicalConfig(
    snapshot: RootSnapshot?,
    enabled: Boolean,
): CanonicalConfig? =
    snapshot
        ?.sections
        ?.get("canonical_config")
        ?.let {
            runCatching { parseCanonicalConfig(it) }.getOrNull()
        }?.copy(debug = enabled)
