package dev.okhsunrog.vpnhide

import android.content.Context
import dev.okhsunrog.vpnhide.generated.HookIds

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

private val STATUS_ERRORS_BY_CODE = HookIds.StatusError.entries.associateBy { it.code.toLong() }

internal fun buildHookDiagnosticsText(
    context: Context,
    shellSnapshot: DebugShellSnapshot,
    counterBaseline: DebugShellSnapshot? = null,
    results: CheckResults? = null,
): String {
    val rootSnapshot = RootSnapshot(shellSnapshot.sections)
    val currentState = buildStatisticsState(rootSnapshot)
    val baselineState = counterBaseline?.let { buildStatisticsState(RootSnapshot(it.sections)) }
    val baselineCounters = baselineState?.toCounterMap().orEmpty()
    val currentCounters = currentState.toCounterMap()
    val statusByBackend = statusByBackend(shellSnapshot)

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
        appendCounterDelta(currentState, baselineCounters)
        appendLine()

        appendLine("=== Native diagnostic checks and expected hooks ===")
        appendNativeChecks(
            context = context,
            results = results,
            statusByBackend = statusByBackend,
            counters = currentCounters,
            baselineCounters = baselineCounters,
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
        val errorName = STATUS_ERRORS_BY_CODE[status.error]?.name ?: "UNKNOWN"
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
    val owned = ownedHooks(backend)
    val installed = owned.filter { installedMask.hasHook(it) }
    val missing = owned.filterNot { installedMask.hasHook(it) }
    appendLine("  installed hooks (${installed.size}/${owned.size}):")
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
            val deltaText = counterDeltaText(row.count, baseline, hasBaseline = baselineCounters.isNotEmpty())
            appendLine(
                "  uid=${row.uid} pkg=${row.packageNames.ifEmpty { listOf("(unknown)") }.joinToString("|")} " +
                    "hook=${row.hook?.hookName ?: row.hookId} count=${formatStatCount(row.count)} delta=$deltaText",
            )
        }
    }
}

private fun StringBuilder.appendNativeChecks(
    context: Context,
    results: CheckResults?,
    statusByBackend: Map<HookIds.Backend, Protocol.Status?>,
    counters: Map<CounterKey, Long>,
    baselineCounters: Map<CounterKey, Long>,
) {
    val byName =
        results
            ?.native
            ?.mapIndexedNotNull { index, result ->
                NATIVE_CHECKS.getOrNull(index)?.id?.let { it to result }
            }.orEmpty()
            .toMap()

    for (spec in NATIVE_CHECKS) {
        val result = byName[spec.id]
        appendLine("${spec.id}: ${result?.name ?: context.getString(spec.labelRes)}")
        appendLine("  result=${result?.passed?.let(::badge) ?: "(not run)"}")
        result?.detail?.takeIf { it.isNotBlank() }?.let { appendLine("  detail=$it") }
        if (spec.expectedHooks.isEmpty()) {
            appendLine("  expected hooks: none registered; this probe is covered outside the hook registry")
        } else {
            appendLine("  expected hooks:")
            spec.expectedHooks.forEach { hook ->
                appendLine("    ${formatHookWithOwners(hook, statusByBackend, counters, baselineCounters)}")
            }
        }
    }
}

private fun badge(passed: Boolean): String = if (passed) "PASS" else "FAIL"

private fun formatHookWithOwners(
    hook: HookIds.Hook,
    statusByBackend: Map<HookIds.Backend, Protocol.Status?>,
    counters: Map<CounterKey, Long>,
    baselineCounters: Map<CounterKey, Long>,
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
    val totalDelta = totalDeltaForHook(hook, counters, baselineCounters)
    return "${formatHook(hook)} [$ownerText, totalDelta=$totalDelta]"
}

private fun totalDeltaForHook(
    hook: HookIds.Hook,
    counters: Map<CounterKey, Long>,
    baselineCounters: Map<CounterKey, Long>,
): String {
    if (baselineCounters.isEmpty()) return "n/a"
    val current = counters.unsignedSumForHook(hook)
    val baseline = baselineCounters.unsignedSumForHook(hook)
    return if (current < baseline) "reset" else "+${formatStatCount(current - baseline)}"
}

private fun counterDeltaText(
    current: Long,
    baseline: Long?,
    hasBaseline: Boolean,
): String {
    if (!hasBaseline) return "n/a"
    if (baseline == null) return "+${formatStatCount(current)}"
    return if (current.toULong() < baseline.toULong()) {
        "reset"
    } else {
        "+${formatStatCount(current.toULong() - baseline.toULong())}"
    }
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

private fun ownedHooks(backend: HookIds.Backend): List<HookIds.Hook> {
    val mask =
        when (backend) {
            HookIds.Backend.KMOD,
            HookIds.Backend.KPM,
            -> HookIds.KERNEL_HOOK_MASK.toLong()

            HookIds.Backend.ZYGISK -> HookIds.ZYGISK_HOOK_MASK.toLong()

            HookIds.Backend.LSPOSED -> HookIds.LSPOSED_HOOK_MASK.toLong()
        }
    return HookIds.Hook.entries.filter { mask.hasHook(it) }
}

private fun hookOwners(hook: HookIds.Hook): List<HookIds.Backend> =
    HookIds.Backend.entries.filter { backend -> hook in ownedHooks(backend) }

private fun Long.hasHook(hook: HookIds.Hook): Boolean = this and (1L shl hook.id) != 0L

private fun formatHook(hook: HookIds.Hook): String = "[${hook.id}] ${hook.hookName} - ${hook.note}"
