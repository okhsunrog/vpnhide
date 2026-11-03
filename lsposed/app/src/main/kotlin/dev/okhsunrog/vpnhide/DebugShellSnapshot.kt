package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreProbeResult
import dev.okhsunrog.vpnhide.core.CoreRoot
import uniffi.vpnhide_core_ffi.CoreProbeFrameKind
import uniffi.vpnhide_core_ffi.coreParseProbeSections
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe as CoreProbe

internal data class DebugShellSnapshot(
    val sections: Map<String, String>,
    val exitCode: Int,
)

/** The bug-report probe core runs (`DebugSnapshot`; core-daemon `device_probes/debug_snapshot.rs`). */
internal fun collectDebugShellSnapshot(): DebugShellSnapshot = snapshotFrom(CoreRoot.collectOneBlocking(CoreProbe.DebugSnapshot), "debug")

/** Hook hit counters from the live backend, without the full snapshot's cost (`HookCounters`). */
internal fun collectHookCounterSnapshot(): DebugShellSnapshot = snapshotFrom(CoreRoot.collectOneBlocking(CoreProbe.HookCounters), "counter")

internal fun snapshotFrom(
    result: CoreProbeResult,
    what: String,
): DebugShellSnapshot {
    val sections = parseDebugShellSnapshot(result.text).toMutableMap()
    if (!result.ok) {
        sections["debug_snapshot_error"] =
            listOfNotNull(sections["debug_snapshot_error"], "core $what snapshot failed with exit=${result.exit}").joinToString("\n")
    }
    if (result.truncated) sections["debug_snapshot_truncated"] = "core cut the $what snapshot at its size or time limit"
    return DebugShellSnapshot(sections = sections, exitCode = result.exit)
}

internal fun parseDebugShellSnapshot(raw: String): Map<String, String> {
    val parsed = coreParseProbeSections(CoreProbeFrameKind.DEBUG, raw)
    val sections = parsed.complete.associate { it.name to it.body }.toMutableMap()
    parsed.error?.let { sections["debug_snapshot_error"] = "snapshot framing invalid: ${it.name}" }
    // A probe that overran core's deadline leaves the last section open (no END
    // marker) and every later section absent. Keep the partial body but flag it,
    // so a bug report shows "cut off here" instead of a silently-missing section.
    parsed.incomplete?.let { incomplete ->
        sections[incomplete.name] =
            (incomplete.body + "\n(TRUNCATED: snapshot cut off before this section completed)").trim()
        sections["debug_snapshot_truncated"] = incomplete.name
    }
    return sections
}
