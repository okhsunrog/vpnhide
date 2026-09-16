package dev.okhsunrog.vpnhide

/**
 * Kotlin side of the vpnhide control/stats wire format (docs/protocol.md §4).
 *
 * Mirrors the freestanding C (`kmod/shared/vpnhide_logic.h`) and the Rust
 * (`crates/protocol`) byte for byte; parity is held by the shared golden
 * vectors (`kmod/shared/protocol_vectors.tsv`), run by ProtocolTest.
 *
 * **Stats and status only.** The app does not speak the `config` payload: it
 * writes canonical JSON and the module's own activator derives the wire from
 * it, so the C and Rust ends are the only config parsers (their parity is what
 * the `cfg|` vectors hold). What lives here is the app reading a backend's
 * stats/status back, plus `LsposedState` emitting the LSPosed layer's own.
 *
 * Numbers are carried as [Long]: `uid`/`hookmask`/`hook_id` are u32 (always
 * non-negative here); `count` is u64 carried as raw bits (use the unsigned
 * formatter/parser), so a full-range counter round-trips exactly.
 */
internal object Protocol {
    /**
     * Version of the **control** protocol (`config`), activator → backend. The
     * app never parses or writes that payload — it is here because the header
     * and its version fuse are the one lexical core all three ports share, and
     * [peekKind] has to classify a control header the same way the C and Rust
     * ends do (the shared `kind` vectors pin exactly that).
     */
    const val CONTROL_VERSION = 2

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

    // ── lexical helpers (§4.1) ────────────────────────────────────────────

    private fun isSep(c: Char) = c == ' ' || c == '\t'

    private fun isAsciiPrintable(c: Char) = c.code in 0x20..0x7e

    /** Split on `\n`, stripping a trailing `\r` per line (CRLF → LF). */
    private fun lines(text: String): List<String> = text.split('\n').map { it.removeSuffix("\r") }

    /** Blank line or a `#` comment — ignored on both header search and records. */
    private fun isIgnorable(line: String): Boolean {
        val s = line.indexOfFirst { !isSep(it) }
        return s < 0 || line[s] == '#'
    }

    /** A line is acceptable only if every byte is printable ASCII or a tab. */
    private fun isAscii(line: String) = line.all { isAsciiPrintable(it) || it == '\t' }

    private fun contentAfterWs(line: String): String = line.substring(line.indexOfFirst { !isSep(it) })

    private fun tokens(content: String): List<String> = content.split(' ', '\t').filter { it.isNotEmpty() }

    /**
     * Parse the one numeric primitive (§4.4): `0x` (mandatory) + ≥1 hex digit,
     * any case; reject if it overflows `bits` (32/64). Returns the value as raw
     * Long bits, or null on any malformation.
     */
    private fun parseHex(
        tok: String,
        bits: Int,
    ): Long? {
        if (tok.length < 3 || tok[0] != '0' || (tok[1] != 'x' && tok[1] != 'X')) return null
        val max = if (bits >= 64) ULong.MAX_VALUE else 0xffffffffuL
        var v = 0uL
        for (c in tok.substring(2)) {
            val d =
                when (c) {
                    in '0'..'9' -> c - '0'
                    in 'a'..'f' -> c - 'a' + 10
                    in 'A'..'F' -> c - 'A' + 10
                    else -> return null
                }.toULong()
            if (v > (max - d) / 16uL) return null // width overflow
            v = v * 16uL + d
        }
        return v.toLong()
    }

    /** Always lowercase out (§4.4: liberal-in / strict-out). Unsigned so a u64
     * value with the high bit set still renders correctly. */
    private fun hex(v: Long): String = "0x" + java.lang.Long.toUnsignedString(v, 16)

    // ── header (§4.2) ─────────────────────────────────────────────────────

    private data class Header(
        val kind: Kind,
        val records: List<String>,
    )

    private fun parseHeader(text: String): Header? {
        val ls = lines(text)
        for (i in ls.indices) {
            val line = ls[i]
            if (isIgnorable(line)) continue // blank / comment before the header is fine
            // first significant line == the mandatory header
            if (!isAscii(line)) return null
            val toks = tokens(contentAfterWs(line))
            if (toks.getOrNull(0) != "vpnhide") return null
            // Digits only, matching the C (`vpnhide_tok_decimal`) and Rust
            // (`parse_decimal`) version parsers. Kotlin's `toIntOrNull()` would
            // otherwise accept a leading `+` (`"+2".toIntOrNull() == 2`), which
            // those reject — a silent three-parser drift the shared `kind`
            // vectors (`vpnhide +1 stats`) now pin.
            val verTok = toks.getOrNull(1) ?: return null
            if (verTok.isEmpty() || verTok.any { it !in '0'..'9' }) return null
            val ver = verTok.toIntOrNull() ?: return null
            val kind =
                when (toks.getOrNull(2)) {
                    "config" -> Kind.CONFIG
                    "stats" -> Kind.STATS
                    "status" -> Kind.STATUS
                    else -> return null
                }
            // The fuse is per protocol: a version only means something once we
            // know which payload it labels.
            val current = if (kind == Kind.CONFIG) CONTROL_VERSION else TELEMETRY_VERSION
            if (ver != current) return null
            return Header(kind, ls.subList(i + 1, ls.size))
        }
        return null
    }

    fun peekKind(text: String): Kind? = parseHeader(text)?.kind

    /** Records that are significant (not blank/comment) and ASCII-clean (§4.5). */
    private inline fun forEachRecord(
        records: List<String>,
        body: (List<String>) -> Unit,
    ) {
        for (line in records) {
            if (isIgnorable(line) || !isAscii(line)) continue
            body(tokens(contentAfterWs(line)))
        }
    }

    // ── stats (§4.3) ──────────────────────────────────────────────────────

    fun parseStats(text: String): List<StatEntry>? {
        val h = parseHeader(text) ?: return null
        if (h.kind != Kind.STATS) return null
        val out = mutableListOf<StatEntry>()
        forEachRecord(h.records) { toks ->
            val uid = toks.getOrNull(0)?.let { parseHex(it, 32) }
            if (uid != null) {
                for (j in 1 until toks.size) {
                    val pair = toks[j].split(':')
                    val hid = pair.getOrNull(0)?.let { parseHex(it, 32) }
                    val cnt = pair.getOrNull(1)?.let { parseHex(it, 64) }
                    if (pair.size == 2 && hid != null && cnt != null) {
                        out += StatEntry(uid, hid, cnt)
                    }
                }
            }
        }
        return out
    }

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

    fun parseStatusFields(text: String): StatusFields? {
        val h = parseHeader(text) ?: return null
        if (h.kind != Kind.STATUS) return null
        var fields = StatusFields()
        forEachRecord(h.records) { toks ->
            if (toks.firstOrNull() == "vpnhide") return fields
            val v = toks.takeIf { it.size == 2 }?.get(1)?.let { parseHex(it, 32) }
            if (v != null) {
                fields =
                    when (toks.firstOrNull()) {
                        "backend" -> fields.copy(backend = v)
                        "kver" -> fields.copy(kver = v)
                        "hooks" -> fields.copy(hooks = v)
                        "error" -> fields.copy(error = v)
                        else -> fields
                    }
            }
        }
        return fields
    }

    fun parseStatus(text: String): Status? =
        parseStatusFields(text)?.let {
            Status(it.backend ?: 0L, it.kver ?: 0L, it.hooks ?: 0L, it.error ?: 0L)
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
