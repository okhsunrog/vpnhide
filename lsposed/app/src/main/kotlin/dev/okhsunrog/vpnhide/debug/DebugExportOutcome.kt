package dev.okhsunrog.vpnhide.debug

import java.io.File

/**
 * The explicit result of an export. A capture whose self-test could not measure is
 * still a written bundle — it carries the reason in [Written.errors] — so the only
 * failure left is one that produced no file at all.
 */
internal sealed interface DebugExportOutcome {
    data class Written(
        val file: File,
        val errors: List<String>,
        val attachments: ArchiveSummaryData? = null,
    ) : DebugExportOutcome

    data class Failed(
        val reason: String,
    ) : DebugExportOutcome
}
