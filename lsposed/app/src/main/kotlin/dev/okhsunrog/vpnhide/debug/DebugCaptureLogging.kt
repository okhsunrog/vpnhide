package dev.okhsunrog.vpnhide.debug

import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.CoreOperationIds
import dev.okhsunrog.vpnhide.core.CoreEffectOutcome
import dev.okhsunrog.vpnhide.core.CoreRoot
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal data class DebugCaptureLoggingSession(
    val id: Long,
    val originalDebug: Boolean,
    val apply: DebugCaptureLoggingStep,
    val restore: DebugCaptureLoggingStep? = null,
    val renewal: Job? = null,
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
        val id = CoreOperationIds.next()
        val original =
            CanonicalConfigRepository.state.value.confirmed
                ?.debug ?: false
        val result = CoreRoot.debugLease(id, acquire = true)
        DebugCaptureLoggingSession(
            id,
            original,
            captureStep(true, result),
            renewal =
                if (result != CoreEffectOutcome.FailedKnown) {
                    CanonicalConfigRepository.processScope.launch(Dispatchers.IO) {
                        while (isActive) {
                            delay(30_000)
                            CoreRoot.debugLease(id, acquire = true)
                        }
                    }
                } else {
                    null
                },
        )
    }

internal suspend fun restoreDebugCaptureLogging(session: DebugCaptureLoggingSession): DebugCaptureLoggingStep =
    withContext(NonCancellable) {
        session.renewal?.cancelAndJoin()
        val result = CoreRoot.debugLease(session.id, acquire = false)
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
    result: CoreEffectOutcome,
) = DebugCaptureLoggingStep(
    requestedEnabled = enabled,
    source = "core_debug_lease",
    rootSnapshotExit = null,
    commandExit = if (result == CoreEffectOutcome.Ok) 0 else -1,
    detail = result.name,
)
