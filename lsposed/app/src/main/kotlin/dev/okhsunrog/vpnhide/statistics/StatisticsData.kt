package dev.okhsunrog.vpnhide.statistics

import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.Protocol
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.backendId
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.indicatesActive
import dev.okhsunrog.vpnhide.nativeStatusSnapshot
import dev.okhsunrog.vpnhide.parseLsposedStateMetadata
import dev.okhsunrog.vpnhide.parsePackageUidMap
import dev.okhsunrog.vpnhide.statusError

internal data class StatisticsState(
    val backends: List<BackendStatistics>,
    val nativeSource: dev.okhsunrog.vpnhide.NativeStatusSnapshot? = null,
    val rootObservationId: Long? = null,
    val rootGeneration: Long? = null,
) {
    val hasAnyData: Boolean = backends.any { it.hasData }
    val activeBackendCount: Int = backends.count { it.isActive }
    val totalRows: Int = backends.sumOf { it.rows.size }
    val totalCount: ULong = backends.fold(0uL) { acc, backend -> acc + backend.totalCount }
}

internal enum class StatisticsUnavailableReason {
    ZygiskNativeStats,
    KpmStatsTruncated,
}

internal data class BackendStatistics(
    val backend: HookIds.Backend,
    val status: Protocol.Status?,
    val metadata: Map<String, String> = emptyMap(),
    val rows: List<StatisticsRow>,
    val unavailableReason: StatisticsUnavailableReason? = null,
    val current: Boolean = false,
) {
    val hasData: Boolean = status != null || rows.isNotEmpty()
    val isActive: Boolean = current
    val totalCount: ULong = rows.fold(0uL) { acc, row -> acc + row.count.toULong() }
    val hookedCount: Int = status?.hooks?.let { java.lang.Long.bitCount(it) } ?: 0
}

internal data class StatisticsRow(
    val uid: Long,
    val packageNames: List<String>,
    val hookId: Long,
    val hook: HookIds.Hook?,
    val count: Long,
)

private val PROTOCOL_KINDS = setOf("config", "stats", "status")
private val HOOKS_BY_ID = HookIds.Hook.entries.associateBy { it.id.toLong() }
private const val KPM_TRUNCATION_MARKER = "# vpnhide truncated"

internal fun parseCounterState(sections: Map<String, String>): StatisticsState {
    val uidPackages = uidPackages(sections["pm_packages"].orEmpty())
    val kmodRaw = sections["kmod_state"].orEmpty()
    val kernelStatus = parseProtocolStatusBlock(kmodRaw)
    val kpmRaw = sections["kpm_state"].orEmpty()
    val lsposedRaw = sections["lsposed_state"].orEmpty()
    val kpmStatsTruncated = kpmRaw.lineSequence().any { it.trim() == KPM_TRUNCATION_MARKER }
    val nativeBackends =
        listOf(
            buildBackendStatistics(
                backend = if (kernelStatus?.backendId == HookIds.Backend.BUILTIN) HookIds.Backend.BUILTIN else HookIds.Backend.KMOD,
                status = kernelStatus,
                stats = parseProtocolStatsBlock(kmodRaw),
                uidPackages = uidPackages,
            ),
            buildBackendStatistics(
                backend = HookIds.Backend.KPM,
                status = parseProtocolStatusBlock(kpmRaw),
                stats = if (kpmStatsTruncated) emptyList() else parseProtocolStatsBlock(kpmRaw),
                uidPackages = uidPackages,
                unavailableReason =
                    StatisticsUnavailableReason.KpmStatsTruncated.takeIf { kpmStatsTruncated },
            ),
        )
    val lsposed =
        buildBackendStatistics(
            backend = HookIds.Backend.LSPOSED,
            status = parseProtocolStatusBlock(lsposedRaw),
            stats = parseProtocolStatsBlock(lsposedRaw),
            metadata = parseLsposedStateMetadata(lsposedRaw),
            uidPackages = uidPackages,
        )

    return StatisticsState(nativeBackends + lsposed)
}

internal fun parseProtocolStatusBlock(raw: String): Protocol.Status? =
    extractProtocolBlock(raw, Protocol.Kind.STATUS)?.let(Protocol::parseStatus)

internal fun parseProtocolStatsBlock(raw: String): List<Protocol.StatEntry> =
    extractProtocolBlock(raw, Protocol.Kind.STATS)?.let(Protocol::parseStats).orEmpty()

internal fun extractProtocolBlock(
    raw: String,
    kind: Protocol.Kind,
): String? {
    val lines = raw.split('\n').map { it.removeSuffix("\r") }
    val start = lines.indexOfFirst { protocolKindOfLine(it) == kind }
    if (start < 0) return null
    val end =
        lines
            .drop(start + 1)
            .indexOfFirst { protocolKindOfLine(it) != null }
            .let { if (it < 0) lines.size else start + 1 + it }
    return lines.subList(start, end).joinToString("\n")
}

internal fun formatStatCount(count: ULong): String =
    count
        .toString()
        .reversed()
        .chunked(3)
        .joinToString(",")
        .reversed()

internal fun formatStatCount(count: Long): String = formatStatCount(count.toULong())

private fun buildBackendStatistics(
    backend: HookIds.Backend,
    status: Protocol.Status?,
    stats: List<Protocol.StatEntry>,
    uidPackages: Map<Long, List<String>>,
    metadata: Map<String, String> = emptyMap(),
    unavailableReason: StatisticsUnavailableReason? = null,
): BackendStatistics =
    BackendStatistics(
        backend = backend,
        status = status,
        metadata = metadata,
        rows = buildStatisticsRows(stats, uidPackages),
        unavailableReason = unavailableReason,
    )

private fun buildStatisticsRows(
    stats: List<Protocol.StatEntry>,
    uidPackages: Map<Long, List<String>>,
): List<StatisticsRow> =
    stats
        .map { entry ->
            StatisticsRow(
                uid = entry.uid,
                packageNames = uidPackages[entry.uid].orEmpty(),
                hookId = entry.hookId,
                hook = HOOKS_BY_ID[entry.hookId],
                count = entry.count,
            )
        }.sortedWith(::compareStatisticsRows)

private fun compareStatisticsRows(
    left: StatisticsRow,
    right: StatisticsRow,
): Int {
    val byCount = java.lang.Long.compareUnsigned(right.count, left.count)
    if (byCount != 0) return byCount
    val byPackage = left.packageNames.joinToString().compareTo(right.packageNames.joinToString())
    if (byPackage != 0) return byPackage
    return left.hookId.compareTo(right.hookId)
}

private fun uidPackages(raw: String): Map<Long, List<String>> {
    val byUid = linkedMapOf<Long, MutableList<String>>()
    parsePackageUidMap(raw).forEach { (pkg, uids) ->
        uids.forEach { uid ->
            byUid.getOrPut(uid.toLong()) { mutableListOf() } += pkg
        }
    }
    return byUid.mapValues { (_, packages) -> packages.distinct().sorted() }
}

private fun protocolKindOfLine(line: String): Protocol.Kind? {
    val trimmed = line.trim()
    if (trimmed.isEmpty() || trimmed.startsWith("#")) return null
    val tokens = trimmed.split(' ', '\t').filter { it.isNotEmpty() }
    if (tokens.size < 3 || tokens[0] != "vpnhide") return null
    if (tokens[1].toIntOrNull() == null || tokens[2] !in PROTOCOL_KINDS) return null
    return when (tokens[2]) {
        "config" -> Protocol.Kind.CONFIG
        "stats" -> Protocol.Kind.STATS
        "status" -> Protocol.Kind.STATUS
        else -> null
    }
}

/** Historical counter identity stays with its own proc snapshot. */
internal data class CounterTelemetry(
    val counters: StatisticsState,
    val nativeSource: dev.okhsunrog.vpnhide.NativeStatusSnapshot,
    val rootObservationId: Long,
    val rootGeneration: Long,
    val javaBootCurrent: Boolean = false,
)

internal fun counterTelemetry(root: RootSnapshot) =
    CounterTelemetry(
        parseCounterState(root.sections),
        root.core.nativeStatusSnapshot(),
        root.observationId,
        root.generation,
        dev.okhsunrog.vpnhide.lsposedHooksActiveThisBoot(
            root.sections["lsposed_state"].orEmpty(),
            root.sections["current_boot_id"].orEmpty(),
        ),
    )

internal fun buildStatisticsState(
    root: RootSnapshot,
    core: dev.okhsunrog.vpnhide.CoreSessionUpdate = root.core,
): StatisticsState = qualifyStatistics(counterTelemetry(root), core.nativeStatusSnapshot())

/** Select current presentation by Core identity, never by stale rows or module order. */
internal fun qualifyStatistics(
    telemetry: CounterTelemetry,
    source: dev.okhsunrog.vpnhide.NativeStatusSnapshot,
): StatisticsState {
    val paired = statisticsSourcesJoined(telemetry, source)
    val native = source.model?.observed.takeIf { paired }
    val backend =
        when (native) {
            NativeBackendId.Kmod -> HookIds.Backend.KMOD
            NativeBackendId.Builtin -> HookIds.Backend.BUILTIN
            NativeBackendId.Kpm -> HookIds.Backend.KPM
            NativeBackendId.Zygisk -> HookIds.Backend.ZYGISK
            null -> null
        }
    val currentNative =
        telemetry.counters.backends.firstOrNull { it.backend == backend }?.let { counters ->
            val runtime = source.model?.runtime
            val sameStatus =
                runtime != null && counters.status?.let { status ->
                    status.backendId == backend && status.kver == runtime.kernelVersion && status.hooks == runtime.installedHooks &&
                        status.error == runtime.statusError
                } == true
            counters.copy(current = sameStatus)
        }
    val zygisk =
        if (paired && source.model?.selected?.backend == NativeBackendId.Zygisk && currentNative == null) {
            BackendStatistics(
                HookIds.Backend.ZYGISK,
                null,
                rows = emptyList(),
                unavailableReason = StatisticsUnavailableReason.ZygiskNativeStats,
            )
        } else {
            null
        }
    val java =
        telemetry.counters.backends.firstOrNull { it.backend == HookIds.Backend.LSPOSED }?.let { counters ->
            counters.copy(current = paired && telemetry.javaBootCurrent && counters.status?.statusError?.indicatesActive == true)
        }
    return StatisticsState(listOfNotNull(currentNative, zygisk, java), source, telemetry.rootObservationId, telemetry.rootGeneration)
}

private fun statisticsSourcesJoined(
    telemetry: CounterTelemetry,
    source: dev.okhsunrog.vpnhide.NativeStatusSnapshot,
): Boolean =
    source.current && telemetry.nativeSource.current && source.epoch == telemetry.nativeSource.epoch &&
        source.connectionGeneration == telemetry.nativeSource.connectionGeneration &&
        source.configRevision == telemetry.nativeSource.configRevision &&
        source.model?.boot == telemetry.nativeSource.model?.boot &&
        source.model?.selected == telemetry.nativeSource.model?.selected &&
        source.model?.delivery == telemetry.nativeSource.model?.delivery &&
        source.model?.observed == telemetry.nativeSource.model?.observed &&
        source.model?.runtime == telemetry.nativeSource.model?.runtime
