package dev.okhsunrog.vpnhide

internal data class DebugShellSnapshot(
    val sections: Map<String, String>,
    val exitCode: Int,
)

private const val DEBUG_SNAPSHOT_BEGIN_PREFIX = "__VPNHIDE_DEBUG_SECTION_BEGIN__:"
private const val DEBUG_SNAPSHOT_END_PREFIX = "__VPNHIDE_DEBUG_SECTION_END__:"

// The batch runs many heavy root commands (per-user package scans, dumpsys
// connectivity, ip route show table all, fib_trie, several sha256sum). On a busy
// device 20s was easy to overrun, which truncated the output mid-section and
// silently dropped that section and every later one. Give it real headroom.
private const val DEBUG_SNAPSHOT_TIMEOUT_SEC: Long = 60
private const val COUNTER_SNAPSHOT_TIMEOUT_SEC: Long = 8

internal fun collectDebugShellSnapshot(): DebugShellSnapshot {
    val (exit, raw) =
        suExec(
            buildDebugShellSnapshotCommand(),
            timeoutSec = DEBUG_SNAPSHOT_TIMEOUT_SEC,
        )
    val sections = parseDebugShellSnapshot(raw).toMutableMap()
    if (exit != 0) {
        sections["debug_snapshot_error"] = "root debug snapshot command failed with exit=$exit"
    }
    return DebugShellSnapshot(sections = sections, exitCode = exit)
}

internal fun collectHookCounterSnapshot(): DebugShellSnapshot {
    val (exit, raw) =
        suExec(
            buildHookCounterSnapshotCommand(),
            timeoutSec = COUNTER_SNAPSHOT_TIMEOUT_SEC,
        )
    val sections = parseDebugShellSnapshot(raw).toMutableMap()
    if (exit != 0) {
        sections["debug_snapshot_error"] = "root counter snapshot command failed with exit=$exit"
    }
    return DebugShellSnapshot(sections = sections, exitCode = exit)
}

internal fun parseDebugShellSnapshot(raw: String): Map<String, String> {
    val parsed =
        parseFramedSections(
            raw = raw,
            beginPrefix = DEBUG_SNAPSHOT_BEGIN_PREFIX,
            endPrefix = DEBUG_SNAPSHOT_END_PREFIX,
            policy =
                FramedSectionParsePolicy(
                    preserveIncomplete = true,
                    discardOnMismatchedEnd = false,
                    trimSectionEnd = true,
                ),
        )
    val sections = parsed.complete.toMutableMap()
    // A command that overran the su timeout leaves the last section open (no END
    // marker) and every later section absent. Keep the partial body but flag it,
    // so a bug report shows "cut off here" instead of a silently-missing section.
    parsed.incomplete?.let { incomplete ->
        sections[incomplete.name] =
            (incomplete.body + "\n(TRUNCATED: snapshot cut off before this section completed)").trim()
        sections["debug_snapshot_truncated"] = incomplete.name
    }
    return sections
}

/**
 * Paths and framing prefixes the debug-side scripts read. One map, because the
 * counter probe is a subset of the same surface.
 */
private fun debugShellVariables(): Map<String, String> =
    mapOf(
        "VPNHIDE_SECTION_BEGIN" to DEBUG_SNAPSHOT_BEGIN_PREFIX,
        "VPNHIDE_SECTION_END" to DEBUG_SNAPSHOT_END_PREFIX,
        "VPNHIDE_PM_USERS_STATUS" to PM_USERS_STATUS_PREFIX,
        "VPNHIDE_PM_USER_BEGIN" to PM_USER_BEGIN_PREFIX,
        "VPNHIDE_PM_USER_END" to PM_USER_END_PREFIX,
        "VPNHIDE_PM_STDERR_TO_STDOUT" to "1",
        "VPNHIDE_KMOD_DIR" to KMOD_MODULE_DIR,
        "VPNHIDE_KPM_DIR" to KPM_MODULE_DIR,
        "VPNHIDE_ZYGISK_DIR" to ZYGISK_MODULE_DIR,
        "VPNHIDE_PORTS_DIR" to PORTS_MODULE_DIR,
        "VPNHIDE_KMOD_ACTIVATOR" to KMOD_ACTIVATOR,
        "VPNHIDE_KPM_ACTIVATOR" to KPM_ACTIVATOR,
        "VPNHIDE_ZYGISK_ACTIVATOR" to ZYGISK_ACTIVATOR,
        "VPNHIDE_PORTS_ACTIVATOR" to PORTS_ACTIVATOR,
        "VPNHIDE_CONFIG_FILE" to CANONICAL_CONFIG_FILE,
        "VPNHIDE_SUPERKEY_FILE" to SUPERKEY_FILE,
        "VPNHIDE_KMOD_LOAD_STATUS" to KMOD_LOAD_STATUS_FILE,
        "VPNHIDE_KMOD_LOAD_DMESG" to KMOD_LOAD_DMESG_FILE,
        "VPNHIDE_ZYGISK_STATUS" to ZYGISK_STATUS_FILE,
        "VPNHIDE_KPM_LOAD_STATUS" to KPM_LOAD_STATUS_FILE,
        "VPNHIDE_PORTS_LOAD_STATUS" to PORTS_LOAD_STATUS_FILE,
        "VPNHIDE_PORTS_LOAD_LOG" to PORTS_LOAD_LOG_FILE,
        "VPNHIDE_PROC_CTL" to PROC_CTL,
        "VPNHIDE_LSPOSED_STATE" to LSPOSED_STATE_FILE,
        "VPNHIDE_LEGACY_HOOK_STATUS" to LEGACY_HOOK_STATUS_FILE,
        "VPNHIDE_LEGACY_SECTIONS" to
            LEGACY_CONFIG_SECTIONS.entries.joinToString(" ") { (section, path) -> "$section=$path" },
    )

/**
 * The bug-report probe. The script lives in `resources/shell/debug_snapshot.sh`,
 * with the shared package inventory concatenated ahead of it; this only supplies
 * the paths and prefixes it reads.
 */
internal fun buildDebugShellSnapshotCommand(): String =
    shellVariables(debugShellVariables()) +
        ShellScripts.load("package_inventory.sh") + "\n" +
        ShellScripts.load("debug_snapshot.sh")

/** The counter-only probe (`resources/shell/hook_counters.sh`): hook hit
 *  counters from the live backend, without the full snapshot's cost. */
internal fun buildHookCounterSnapshotCommand(): String = shellVariables(debugShellVariables()) + ShellScripts.load("hook_counters.sh")
