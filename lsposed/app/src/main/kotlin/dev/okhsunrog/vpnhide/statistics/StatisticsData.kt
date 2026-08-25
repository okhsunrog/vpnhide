package dev.okhsunrog.vpnhide.statistics

import dev.okhsunrog.vpnhide.NativeBackendId
import dev.okhsunrog.vpnhide.Protocol
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.activeNativeBackendId
import dev.okhsunrog.vpnhide.backendId
import dev.okhsunrog.vpnhide.detectNativeBackendStates
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.indicatesActive
import dev.okhsunrog.vpnhide.parseLsposedStateMetadata
import dev.okhsunrog.vpnhide.parsePackageUidMap
import dev.okhsunrog.vpnhide.statusError

internal data class StatisticsState(
    val backends: List<BackendStatistics>,
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
) {
    val hasData: Boolean = status != null || rows.isNotEmpty()
    val isActive: Boolean = hasData || unavailableReason != null
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

internal fun buildStatisticsState(snapshot: RootSnapshot): StatisticsState {
    val uidPackages = uidPackages(snapshot.sections["pm_packages"].orEmpty())
    val kmodRaw = snapshot.sections["kmod_state"].orEmpty()
    val kpmRaw = snapshot.sections["kpm_state"].orEmpty()
    val lsposedRaw = snapshot.sections["lsposed_state"].orEmpty()
    val kpmStatsTruncated = kpmRaw.lineSequence().any { it.trim() == KPM_TRUNCATION_MARKER }
    val activeNativeBackendId = detectNativeBackendStates(snapshot.sections).activeId
    val nativeBackends =
        listOf(
            buildBackendStatistics(
                backend = HookIds.Backend.KMOD,
                status = parseProtocolStatusBlock(kmodRaw),
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

    return StatisticsState(
        listOfNotNull(
            selectActiveNativeStatisticsBackend(nativeBackends, activeNativeBackendId),
            lsposed,
        ),
    )
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

private fun selectActiveNativeStatisticsBackend(
    backends: List<BackendStatistics>,
    activeNativeBackendId: NativeBackendId?,
): BackendStatistics? =
    backends.firstOrNull(BackendStatistics::isActiveNativeStatisticsBackend)
        ?: activeNativeBackendId
            ?.takeIf { it == NativeBackendId.Zygisk }
            ?.let {
                BackendStatistics(
                    backend = HookIds.Backend.ZYGISK,
                    status = null,
                    rows = emptyList(),
                    unavailableReason = StatisticsUnavailableReason.ZygiskNativeStats,
                )
            }

private fun BackendStatistics.isActiveNativeStatisticsBackend(): Boolean {
    if (rows.isNotEmpty()) return true
    val status = status ?: return false
    return status.backendId == backend && status.statusError?.indicatesActive == true
}

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
