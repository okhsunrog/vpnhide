package dev.okhsunrog.vpnhide

/** Shared value types and LSPosed telemetry producer. App readers use [ProtocolReader]
 * and the shared Rust wire grammar; formatting here must run without app native code.
 */
internal object Protocol {
    /**
     * Version of the **telemetry** protocol (`stats` + `status`), backend →
     * app. This is the one the app actually speaks, in both directions:
     * `LsposedState` emits its own layer's snapshots and the dashboard parses a
     * native backend's. Moving it means shipping the APK in step with every
     * module, which is why it is versioned apart from control (§3).
     */
    const val TELEMETRY_VERSION = 1

    enum class Kind { CONFIG, STATS, STATUS }

    data class StatEntry(
        val uid: Long,
        val hookId: Long,
        val count: Long,
    )

    data class Status(
        val backend: Long,
        val kver: Long,
        val hooks: Long,
        val error: Long,
    )

    /**
     * Self-documenting banner a read endpoint prepends to its snapshot
     * (§OPEN-7). It's a `#` comment line, ignored by every parser.
     */
    const val READ_BANNER = "# vpnhide v1 — a WRITE replaces ENTIRE state; this read is status+stats\n"

    /** Unsigned wire formatting stays local to the injected LSPosed producer. */
    private fun hex(v: Long): String = "0x" + java.lang.Long.toUnsignedString(v, 16)

    fun formatStats(entries: List<StatEntry>): String =
        buildString {
            append("vpnhide ").append(TELEMETRY_VERSION).append(" stats\n")
            var i = 0
            while (i < entries.size) {
                val uid = entries[i].uid
                append(hex(uid))
                while (i < entries.size && entries[i].uid == uid) {
                    append(' ').append(hex(entries[i].hookId)).append(':').append(hex(entries[i].count))
                    i++
                }
                append('\n')
            }
        }

    // ── status (§4.3) ─────────────────────────────────────────────────────

    data class StatusFields(
        val backend: Long? = null,
        val kver: Long? = null,
        val hooks: Long? = null,
        val error: Long? = null,
    ) {
        fun complete(): Status? {
            return Status(backend ?: return null, kver ?: return null, hooks ?: return null, error ?: return null)
        }
    }

    fun formatStatus(s: Status): String =
        "vpnhide $TELEMETRY_VERSION status\n" +
            "backend ${hex(s.backend)}\nkver ${hex(s.kver)}\nhooks ${hex(s.hooks)}\nerror ${hex(s.error)}\n"

    /**
     * Clamp a fully-serialised snapshot to `outlen` bytes on a line boundary
     * (§7.2). Whole thing fits ⇒ returns its length (ends in `\n`, complete);
     * else the largest run of complete lines minus the trailing `\n` (the
     * missing newline is the truncation signal).
     */
    fun clampToLine(
        buf: String,
        outlen: Int,
    ): Int {
        if (buf.length <= outlen) return buf.length
        var p = outlen
        while (p > 0 && buf[p - 1] != '\n') p--
        return if (p > 0) p - 1 else 0
    }
}
