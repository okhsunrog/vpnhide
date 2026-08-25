package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.Layer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Runs the language-independent golden vectors (kmod/shared/protocol_vectors.tsv)
 * against the Kotlin [Protocol] — the same file the C and Rust ports run, so a
 * divergence on any covered corner fails here (docs/protocol.md §8 Layer 1).
 */
class ProtocolTest {
    private fun vectorsFile(): File {
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        repeat(8) {
            val f = File(dir, "kmod/shared/protocol_vectors.tsv")
            if (f.isFile) return f
            dir = dir?.parentFile
        }
        error("protocol_vectors.tsv not found from ${System.getProperty("user.dir")}")
    }

    /** Decode the `\n \t \r \\ \xNN` escapes used in the vectors file. */
    private fun decode(s: String): String =
        buildString {
            var i = 0
            while (i < s.length) {
                if (s[i] == '\\' && i + 1 < s.length) {
                    when (s[i + 1]) {
                        'n' -> {
                            append('\n')
                            i += 2
                        }

                        't' -> {
                            append('\t')
                            i += 2
                        }

                        'r' -> {
                            append('\r')
                            i += 2
                        }

                        '\\' -> {
                            append('\\')
                            i += 2
                        }

                        'x' -> {
                            if (i + 3 < s.length) {
                                append(s.substring(i + 2, i + 4).toInt(16).toChar())
                                i += 4
                            } else {
                                append(s[i])
                                i++
                            }
                        }

                        else -> {
                            append(s[i])
                            i++
                        }
                    }
                } else {
                    append(s[i])
                    i++
                }
            }
        }

    private fun hexToLong(s: String): Long = java.lang.Long.parseUnsignedLong(s.trim().removePrefix("0x"), 16)

    private fun runKind(
        input: String,
        expect: String,
    ) {
        val got =
            when (Protocol.peekKind(decode(input))) {
                Protocol.Kind.CONFIG -> "CONFIG"
                Protocol.Kind.STATS -> "STATS"
                Protocol.Kind.STATUS -> "STATUS"
                null -> "INVALID"
            }
        assertEquals("kind <$input>", expect, got)
    }

    private fun runStats(
        entries: String,
        expect: String,
    ) {
        val e =
            decode(entries).split(';').filter { it.isNotEmpty() }.map { grp ->
                val p = grp.split(',')
                Protocol.StatEntry(hexToLong(p[0]), hexToLong(p[1]), hexToLong(p[2]))
            }
        assertEquals("stats", decode(expect), Protocol.formatStats(e))
    }

    private fun runStatus(
        fields: String,
        expect: String,
    ) {
        val f = decode(fields).split(',')
        val s = Protocol.Status(hexToLong(f[0]), hexToLong(f[1]), hexToLong(f[2]), hexToLong(f[3]))
        assertEquals("status", decode(expect), Protocol.formatStatus(s))
    }

    private fun runClamp(
        full: String,
        outlen: String,
        expect: String,
    ) {
        val b = decode(full)
        val n = Protocol.clampToLine(b, outlen.toInt())
        assertEquals("clamp", decode(expect), b.substring(0, n))
    }

    @Test
    fun goldenVectors() {
        var count = 0
        vectorsFile().forEachLine { line ->
            if (line.isEmpty() || line.startsWith("#")) return@forEachLine
            val f = line.split('|')
            // `cfg` vectors cover the config payload, which the app does not
            // parse — the C and Rust ends are its only readers and hold that
            // parity between themselves.
            when (f[0]) {
                "cfg" -> return@forEachLine
                "kind" -> runKind(f[1], f[2])
                "stats" -> runStats(f[1], f[2])
                "status" -> runStatus(f[1], f[2])
                "clamp" -> runClamp(f[1], f[2], f[3])
                else -> error("unrecognised vector: $line")
            }
            count++
        }
        assertTrue("expected the full non-cfg vector set, ran $count", count >= 16)
    }
}
