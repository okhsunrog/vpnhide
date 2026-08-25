package dev.okhsunrog.vpnhide.debug

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.VpnHideLog
import dev.okhsunrog.vpnhide.parseCanonicalConfig
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicLong

private const val DEBUG_CAPTURE_OBSERVER_DELAY_MS = 250L

private val debugCaptureMutex = Mutex()
private val nextDebugCaptureSessionId = AtomicLong(1L)
private val activeDebugCaptureSessions = linkedSetOf<Long>()

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
        debugCaptureMutex.withLock {
            val sessionId = nextDebugCaptureSessionId.getAndIncrement()
            val snapshot =
                runCatching { RootSnapshotCache.getOrLoad() }.getOrNull()
                    ?: RootSnapshotCache.snapshot.value
            val originalDebug = debugFromCanonicalSnapshot(snapshot)
            activeDebugCaptureSessions += sessionId
            val apply =
                runCatching {
                    applyCanonicalDebugToggle(enabled = true, sourceSnapshot = snapshot)
                }.getOrElse { failureStep(requestedEnabled = true, detail = failureDetail(failure = it)) }
            return@withLock DebugCaptureLoggingSession(
                id = sessionId,
                originalDebug = originalDebug,
                apply = apply,
            )
        }
    }

internal suspend fun restoreDebugCaptureLogging(session: DebugCaptureLoggingSession): DebugCaptureLoggingStep? =
    withContext(NonCancellable) {
        debugCaptureMutex.withLock {
            if (!activeDebugCaptureSessions.remove(session.id)) {
                return@withLock DebugCaptureLoggingStep(
                    requestedEnabled = session.originalDebug,
                    source = "capture_session_not_active",
                    rootSnapshotExit = null,
                    commandExit = null,
                    detail = "sessionId=${session.id} was already restored or never registered",
                )
            }

            if (activeDebugCaptureSessions.isNotEmpty()) {
                return@withLock DebugCaptureLoggingStep(
                    requestedEnabled = true,
                    source = "restore_deferred_active_capture",
                    rootSnapshotExit = null,
                    commandExit = null,
                    detail = "activeCaptureSessions=${activeDebugCaptureSessions.size}",
                )
            }

            val restoreDebug =
                RootSnapshotCache.snapshot.value
                    ?.let { parseTargetsSnapshot(it).canonicalConfig?.debugSwitch }
                    ?: session.originalDebug
            if (restoreDebug == VpnHideLog.enabled && restoreDebug == session.apply.requestedEnabled) {
                return@withLock null
            }
            return@withLock runCatching {
                applyCanonicalDebugToggle(enabled = restoreDebug, sourceSnapshot = RootSnapshotCache.snapshot.value)
            }.getOrElse { failureStep(requestedEnabled = restoreDebug, detail = failureDetail(failure = it)) }
        }
    }

private fun failureStep(
    requestedEnabled: Boolean,
    detail: String,
): DebugCaptureLoggingStep =
    DebugCaptureLoggingStep(
        requestedEnabled = requestedEnabled,
        source = "canonical_missing_or_unavailable",
        rootSnapshotExit = null,
        commandExit = null,
        detail = detail,
    )

private fun failureDetail(failure: Throwable): String =
    failure.message?.ifBlank { failure::class.java.simpleName } ?: failure::class.java.simpleName

private suspend fun applyCanonicalDebugToggle(
    enabled: Boolean,
    sourceSnapshot: RootSnapshot?,
): DebugCaptureLoggingStep {
    val (snapshot, rootSnapshotExit) =
        if (sourceSnapshot != null) {
            sourceSnapshot to 0
        } else {
            runCatching { RootSnapshotCache.refresh() }.getOrNull() to null
        }
    val resolvedSnapshot = snapshot ?: RootSnapshotCache.snapshot.value
    val canonical = debugToggledCanonicalConfig(resolvedSnapshot, enabled)
    if (canonical == null) {
        VpnHideLog.enabled = enabled
        refreshDebugCaptureState()
        delay(DEBUG_CAPTURE_OBSERVER_DELAY_MS)
        return DebugCaptureLoggingStep(
            requestedEnabled = enabled,
            source = "canonical_unavailable",
            rootSnapshotExit = rootSnapshotExit,
            commandExit = null,
            detail = "canonical config is unavailable or malformed",
        )
    }

    val result = CanonicalConfigRepository.commit(canonical)

    VpnHideLog.enabled = enabled
    refreshDebugCaptureState()
    delay(DEBUG_CAPTURE_OBSERVER_DELAY_MS)
    return DebugCaptureLoggingStep(
        requestedEnabled = enabled,
        source = "canonical_debug_only",
        rootSnapshotExit = rootSnapshotExit,
        commandExit = result.exitCode,
        detail = result.output.trim(),
    )
}

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

// Atomic refresh (not invalidate): after commit() already refreshed the caches in
// place, dropping to null here would just re-open the flicker window this whole
// change exists to close. In the canonical == null fallback path (no persist ran)
// this refresh is the coherence step instead.
private suspend fun refreshDebugCaptureState() {
    CanonicalConfigRepository.refreshDerivedCaches()
}
