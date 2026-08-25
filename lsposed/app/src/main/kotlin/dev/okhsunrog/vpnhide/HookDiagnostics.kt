package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.statistics.StatisticsState
import dev.okhsunrog.vpnhide.statistics.buildStatisticsState
import dev.okhsunrog.vpnhide.statistics.formatStatCount
import dev.okhsunrog.vpnhide.statistics.parseProtocolStatusBlock

private data class CounterKey(
    val backend: HookIds.Backend,
    val uid: Long,
    val hookId: Long,
)

private val BACKEND_STATE_SECTIONS =
    linkedMapOf(
        HookIds.Backend.KMOD to "kmod_state",
        HookIds.Backend.KPM to "kpm_state",
        HookIds.Backend.ZYGISK to "zygisk_status",
        HookIds.Backend.LSPOSED to "lsposed_state",
    )

internal fun buildHookDiagnosticsText(
    context: Context,
    shellSnapshot: DebugShellSnapshot,
    counterBaseline: DebugShellSnapshot? = null,
    report: DiagnosticReport? = null,
): String {
    val rootSnapshot = RootSnapshot(shellSnapshot.sections)
    val currentState = buildStatisticsState(rootSnapshot)
    val baselineState = counterBaseline?.let { buildStatisticsState(RootSnapshot(it.sections)) }
    val baselineCounters = baselineState?.toCounterMap().orEmpty()
    val currentCounters = currentState.toCounterMap()
    // Whether a baseline was actually captured — NOT whether it had counter rows.
    // A fresh boot with no prior traffic captures a valid but empty baseline, and
    // deriving "has baseline" from baselineCounters.isNotEmpty() would then report
    // every delta as n/a, defeating the before/after report.
    val hasBaseline = counterBaseline != null
    val statusByBackend = statusByBackend(shellSnapshot)
    val nativeChecksById =
        report
            ?.native
            ?.checks
            ?.associateBy { it.id }
            .orEmpty()

    return buildString {
        appendLine("Hook diagnostics")
        appendLine("App package: ${context.packageName}")
        appendLine("Current snapshot exit: ${shellSnapshot.exitCode}")
        appendLine("Baseline snapshot exit: ${counterBaseline?.exitCode?.toString() ?: "(not captured)"}")
        appendLine()

        appendLine("=== Backend status masks ===")
        for (backend in HookIds.Backend.entries) {
            appendBackendStatus(backend, statusByBackend[backend], shellSnapshot)
            appendLine()
        }

        appendLine("=== Counter delta after diagnostics ===")
        appendCounterDelta(currentState, baselineCounters, hasBaseline)
        appendLine()

        appendLine("=== Native diagnostic checks and expected hooks ===")
        appendNativeChecks(
            context = context,
            checksById = nativeChecksById,
            statusByBackend = statusByBackend,
            counters = currentCounters,
            baselineCounters = baselineCounters,
            hasBaseline = hasBaseline,
        )
    }.trimEnd()
}

private fun statusByBackend(shellSnapshot: DebugShellSnapshot): Map<HookIds.Backend, Protocol.Status?> =
    BACKEND_STATE_SECTIONS.mapValues { (_, section) ->
        parseProtocolStatusBlock(shellSnapshot.sections[section].orEmpty())
    }

private fun StringBuilder.appendBackendStatus(
    backend: HookIds.Backend,
    status: Protocol.Status?,
    shellSnapshot: DebugShellSnapshot,
) {
    appendLine("${backend.name}:")
    if (status == null) {
        appendLine("  status: not reported in ${BACKEND_STATE_SECTIONS[backend]}")
        if (backend == HookIds.Backend.ZYGISK) {
            appendLine("  note: Zygisk currently publishes heartbeat/config evidence but no counters")
        }
    } else {
        val errorName = status.statusError?.name ?: "UNKNOWN"
        appendLine("  status.backend=${status.backend}")
        appendLine("  status.kver=0x${status.kver.toString(16)}")
        appendLine("  status.hooks=0x${status.hooks.toString(16)}")
        appendLine("  status.error=$errorName(${status.error})")
        appendInstalledHooks(backend, status.hooks)
    }
    if (backend == HookIds.Backend.LSPOSED) {
        val meta = parseLsposedStateMetadata(shellSnapshot.sections["lsposed_state"].orEmpty())
        if (meta.isNotEmpty()) {
            appendLine("  metadata:")
            meta.toSortedMap().forEach { (key, value) ->
                appendLine("    $key=$value")
            }
        }
    }
}

private fun StringBuilder.appendInstalledHooks(
    backend: HookIds.Backend,
    installedMask: Long,
) {
    val installed = ownedHooks(backend).intersect(hooksInMask(installedMask))
    val expected = expectedInstalledHooks(backend, installed)
    val missing = expected - installed
    appendLine("  installed hooks (${installed.size}/${expected.size} expected):")
    if (installed.isEmpty()) {
        appendLine("    (none)")
    } else {
        installed.forEach { appendLine("    ${formatHook(it)}") }
    }
    if (missing.isNotEmpty()) {
        appendLine("  missing owned hooks:")
        missing.forEach { appendLine("    ${formatHook(it)}") }
    }
}

private fun StringBuilder.appendCounterDelta(
    currentState: StatisticsState,
    baselineCounters: Map<CounterKey, Long>,
    hasBaseline: Boolean,
) {
    if (currentState.backends.none { it.rows.isNotEmpty() }) {
        appendLine("(no counter rows)")
        return
    }
    for (backend in currentState.backends) {
        appendLine("${backend.backend.name}:")
        if (backend.rows.isEmpty()) {
            appendLine("  (no counter rows)")
            continue
        }
        backend.rows.forEach { row ->
            val key = CounterKey(backend.backend, row.uid, row.hookId)
            val baseline = baselineCounters[key]
            val deltaText = counterDeltaText(row.count, baseline, hasBaseline = hasBaseline)
            appendLine(
                "  uid=${row.uid} pkg=${row.packageNames.ifEmpty { listOf("(unknown)") }.joinToString("|")} " +
                    "hook=${row.hook?.hookName ?: row.hookId} count=${formatStatCount(row.count)} delta=$deltaText",
            )
        }
    }
}

private fun StringBuilder.appendNativeChecks(
    context: Context,
    checksById: Map<String, DiagnosticCheck>,
    statusByBackend: Map<HookIds.Backend, Protocol.Status?>,
    counters: Map<CounterKey, Long>,
    baselineCounters: Map<CounterKey, Long>,
    hasBaseline: Boolean,
) {
    for (spec in NATIVE_CHECKS) {
        val check = checksById[spec.id]
        appendLine("${spec.id}: ${check?.label ?: context.getString(spec.labelRes)}")
        appendLine("  result=${check?.outcome?.token() ?: "(not run)"}")
        check?.appDetail?.takeIf { it.isNotBlank() }?.let { appendLine("  detail=$it") }
        check?.groundTruthDetail?.let { appendLine("  root=$it") }
        if (spec.expectedHooks.isEmpty()) {
            appendLine("  expected hooks: none registered; this probe is covered outside the hook registry")
        } else {
            appendLine("  expected hooks:")
            spec.expectedHooks.forEach { hook ->
                appendLine("    ${formatHookWithOwners(hook, statusByBackend, counters, baselineCounters, hasBaseline)}")
            }
        }
    }
}

private fun formatHookWithOwners(
    hook: HookIds.Hook,
    statusByBackend: Map<HookIds.Backend, Protocol.Status?>,
    counters: Map<CounterKey, Long>,
    baselineCounters: Map<CounterKey, Long>,
    hasBaseline: Boolean,
): String {
    val ownerText =
        hookOwners(hook)
            .joinToString(",") { backend ->
                val status = statusByBackend[backend]
                when {
                    status == null -> "${backend.name}:n/a"
                    status.hooks.hasHook(hook) -> "${backend.name}:installed"
                    else -> "${backend.name}:missing"
                }
            }.ifBlank { "owner:n/a" }
    val totalDelta = totalDeltaForHook(hook, counters, baselineCounters, hasBaseline)
    return "${formatHook(hook)} [$ownerText, totalDelta=$totalDelta]"
}

private fun totalDeltaForHook(
    hook: HookIds.Hook,
    counters: Map<CounterKey, Long>,
    baselineCounters: Map<CounterKey, Long>,
    hasBaseline: Boolean,
): String =
    if (!hasBaseline) {
        "n/a"
    } else {
        unsignedDeltaText(counters.unsignedSumForHook(hook), baselineCounters.unsignedSumForHook(hook))
    }

internal fun counterDeltaText(
    current: Long,
    baseline: Long?,
    hasBaseline: Boolean,
): String = if (!hasBaseline) "n/a" else unsignedDeltaText(current.toULong(), baseline?.toULong())

/** "reset" when the counter ran backwards (device/backend reset), else "+delta"
 * unsigned. A null [baseline] means the row is new since the baseline → "+current". */
private fun unsignedDeltaText(
    current: ULong,
    baseline: ULong?,
): String =
    when {
        baseline == null -> "+${formatStatCount(current)}"
        current < baseline -> "reset"
        else -> "+${formatStatCount(current - baseline)}"
    }

private fun Map<CounterKey, Long>.unsignedSumForHook(hook: HookIds.Hook): ULong =
    filterKeys { key -> key.hookId == hook.id.toLong() }
        .values
        .fold(0uL) { acc, value -> acc + value.toULong() }

private fun StatisticsState.toCounterMap(): Map<CounterKey, Long> =
    backends
        .flatMap { backend ->
            backend.rows.map { row ->
                CounterKey(
                    backend = backend.backend,
                    uid = row.uid,
                    hookId = row.hookId,
                ) to row.count
            }
        }.toMap()

private fun hookOwners(hook: HookIds.Hook): List<HookIds.Backend> =
    HookIds.Backend.entries.filter { backend -> hook in ownedHooks(backend) }

private fun formatHook(hook: HookIds.Hook): String = "[${hook.id}] ${hook.hookName} - ${hook.note}"
