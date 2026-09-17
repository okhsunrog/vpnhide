package dev.okhsunrog.vpnhide.diagnostics

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.collectLatest

/**
 * How long a newly owed confirmation waits before it is requested: enough to
 * coalesce the separate emissions one change produces (the routing observation,
 * then the root snapshot it folds), too short to be seen. The pending window is
 * worded as the run it becomes, so it costs no flicker either way.
 */
internal const val CONFIRMATION_SETTLE_MS = 300L

/** The least spacing between two automatic requests, so a flapping VPN cannot keep the suite busy. */
internal const val CONFIRMATION_MIN_INTERVAL_MS = 5_000L

/**
 * The automatic confirmation the presentation owes right now, as the key it is
 * owed for, or null. This is the one rule behind every automatic run after
 * startup: it replaces edge detection (a gate value seen to change, a foreground
 * return, a session compared with the previous sample), which loses the edge
 * whenever the baseline is not there to compare with.
 *
 * A run is owed when this app is eligible and its measurable world has a key
 * that nothing covers: neither the presented measurement (it still applies),
 * nor the latest attempt (it was taken under that key, whatever its outcome:
 * a failure is answered by the user's Retry, not by a loop), nor [claimed]
 * (the key the owner already asked for, so an attempt that failed before it
 * had a context cannot be asked for again). A run in flight makes no claim, so
 * nothing is owed beside it; a quarantined probe cannot run.
 */
internal fun owedConfirmation(
    presentation: DiagnosticPresentation,
    claimed: MeasurementKey?,
): MeasurementKey? {
    val key = presentation.currentKey ?: return null
    if (presentation.eligibility != DiagnosticEligibility.Eligible) return null
    if (presentation.activeRunId != null || presentation.probeUnavailable) return null
    if (presentation.measurement?.context?.key == key) return null
    val attempt = presentation.lastAttempt
    if (attempt != null && !attempt.blocked && attempt.measurement?.context?.key == key) return null
    if (claimed == key) return null
    return key
}

/**
 * Own the automatic confirmations of a presentation flow: whenever one is
 * pending, wait for the conditions to settle, then [request] it once. A newer
 * presentation restarts the wait; the request's acceptance is what the caller
 * turns into the claimed key, so a request that was not admitted leaves the
 * presentation pending and the next change asks again. [clock] is on the same
 * base as [wait]'s durations; both are injected so a test can pin the schedule.
 */
@OptIn(ExperimentalCoroutinesApi::class)
internal suspend fun confirmMeasurements(
    presentations: StateFlow<DiagnosticPresentation>,
    request: (MeasurementKey) -> Boolean,
    clock: () -> Long,
    wait: suspend (Long) -> Unit = { delay(it) },
) {
    var lastRequestedAt: Long? = null
    presentations.collectLatest { presentation ->
        if (!presentation.confirmationPending) return@collectLatest
        val key = presentation.currentKey ?: return@collectLatest
        wait(confirmationHoldOff(clock(), lastRequestedAt))
        // The wait may have returned without suspending; ask only if the latest
        // presentation still owes this key, not the one that started the wait.
        val latest = presentations.value
        if (!latest.confirmationPending || latest.currentKey != key) return@collectLatest
        if (request(key)) lastRequestedAt = clock()
    }
}

/** The settle window, stretched to keep the minimum spacing from the previous request. */
internal fun confirmationHoldOff(
    now: Long,
    lastRequestedAt: Long?,
): Long {
    val spacing = lastRequestedAt?.let { it + CONFIRMATION_MIN_INTERVAL_MS - now } ?: 0L
    return maxOf(CONFIRMATION_SETTLE_MS, spacing)
}
