package dev.okhsunrog.vpnhide

import android.content.Context
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
    val persistedEnabled: Boolean,
    val runtimeWasEnabled: Boolean,
    val apply: DebugCaptureLoggingStep,
    val restore: DebugCaptureLoggingStep? = null,
) {
    val forced: Boolean
        get() = !persistedEnabled || !runtimeWasEnabled

    fun withRestore(restore: DebugCaptureLoggingStep?): DebugCaptureLoggingSession = copy(restore = restore)

    fun toText(): String =
        buildString {
            appendLine("sessionId=$id")
            appendLine("persistedEnabled=$persistedEnabled")
            appendLine("runtimeWasEnabled=$runtimeWasEnabled")
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

internal suspend fun beginDebugCaptureLogging(context: Context): DebugCaptureLoggingSession =
    withContext(NonCancellable) {
        debugCaptureMutex.withLock {
            val sessionId = nextDebugCaptureSessionId.getAndIncrement()
            val persisted = isEnabledInPrefs(context)
            val runtime = VpnHideLog.enabled
            val activeBeforeBegin = activeDebugCaptureSessions.size
            activeDebugCaptureSessions += sessionId
            try {
                val apply =
                    if (activeBeforeBegin == 0) {
                        applyDebugLoggingForCapture(enabled = true)
                    } else {
                        VpnHideLog.enabled = true
                        DebugCaptureLoggingStep(
                            requestedEnabled = true,
                            source = "already_forced_by_active_capture",
                            rootSnapshotExit = null,
                            commandExit = null,
                            detail = "activeCaptureSessions=$activeBeforeBegin",
                        )
                    }
                DebugCaptureLoggingSession(
                    id = sessionId,
                    persistedEnabled = persisted,
                    runtimeWasEnabled = runtime,
                    apply = apply,
                )
            } catch (t: Throwable) {
                activeDebugCaptureSessions -= sessionId
                if (activeDebugCaptureSessions.isEmpty()) {
                    runCatching { applyDebugLoggingForCapture(enabled = isEnabledInPrefs(context)) }
                }
                throw t
            }
        }
    }

internal suspend fun restoreDebugCaptureLogging(
    context: Context,
    session: DebugCaptureLoggingSession,
): DebugCaptureLoggingStep? =
    withContext(NonCancellable) {
        debugCaptureMutex.withLock {
            val target = isEnabledInPrefs(context)
            if (!activeDebugCaptureSessions.remove(session.id)) {
                return@withLock DebugCaptureLoggingStep(
                    requestedEnabled = target,
                    source = "capture_session_not_active",
                    rootSnapshotExit = null,
                    commandExit = null,
                    detail = "sessionId=${session.id} was already restored or never registered",
                )
            }

            if (activeDebugCaptureSessions.isNotEmpty()) {
                VpnHideLog.enabled = true
                return@withLock DebugCaptureLoggingStep(
                    requestedEnabled = true,
                    source = "restore_deferred_active_capture",
                    rootSnapshotExit = null,
                    commandExit = null,
                    detail =
                        "targetAfterLastCapture=$target activeCaptureSessions=${activeDebugCaptureSessions.size}",
                )
            }

            if (target == session.apply.requestedEnabled && VpnHideLog.enabled == target) {
                null
            } else {
                applyDebugLoggingForCapture(enabled = target)
            }
        }
    }

private suspend fun applyDebugLoggingForCapture(enabled: Boolean): DebugCaptureLoggingStep {
    VpnHideLog.enabled = enabled
    val rootSnapshotResult = runCatching { RootSnapshotCache.refresh() }
    val snapshot = rootSnapshotResult.getOrNull()
    if (snapshot == null) {
        val message = rootSnapshotResult.exceptionOrNull()?.message.orEmpty()
        return fallbackDebugRuntimeApply(enabled, message)
    }

    val canonical = debugToggledCanonicalConfig(snapshot, enabled)
    if (canonical == null) {
        return fallbackDebugRuntimeApply(enabled, "canonical config unavailable and target snapshot fallback failed")
    }

    val cmd =
        listOf(
            buildCanonicalConfigWriteCommand(canonical.config),
            ConfigChannels.reconcileCommand(),
        ).joinToString(" ; ")
    val (exit, out) = suExec(cmd)
    invalidateDebugCaptureState()
    delay(DEBUG_CAPTURE_OBSERVER_DELAY_MS)
    return DebugCaptureLoggingStep(
        requestedEnabled = enabled,
        source = canonical.source,
        rootSnapshotExit = 0,
        commandExit = exit,
        detail = out.trim(),
    )
}

private suspend fun fallbackDebugRuntimeApply(
    enabled: Boolean,
    reason: String,
): DebugCaptureLoggingStep {
    VpnHideLog.enabled = enabled
    invalidateDebugCaptureState()
    delay(DEBUG_CAPTURE_OBSERVER_DELAY_MS)
    return DebugCaptureLoggingStep(
        requestedEnabled = enabled,
        source = "app_process_only_fallback",
        rootSnapshotExit = null,
        commandExit = null,
        detail = reason.ifBlank { "root snapshot unavailable" },
    )
}

internal data class DebugCanonicalUpdate(
    val config: CanonicalConfig,
    val source: String,
)

internal fun debugToggledCanonicalConfig(
    snapshot: RootSnapshot,
    enabled: Boolean,
): DebugCanonicalUpdate? {
    val existing =
        runCatching { parseCanonicalConfig(snapshot.sections["canonical_config"].orEmpty()) }
            .getOrNull()
    if (existing != null) {
        return DebugCanonicalUpdate(existing.copy(debug = enabled), "canonical_debug_only")
    }

    return runCatching {
        DebugCanonicalUpdate(
            buildCanonicalConfigFromTargetsSnapshot(parseTargetsSnapshot(snapshot), debug = enabled),
            "targets_snapshot_fallback",
        )
    }.getOrNull()
}

private fun invalidateDebugCaptureState() {
    RootSnapshotCache.invalidate()
    TargetsCache.invalidate()
    DashboardCache.invalidate()
}
