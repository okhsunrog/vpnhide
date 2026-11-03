package dev.okhsunrog.vpnhide.migration

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.toCanonical
import uniffi.vpnhide_core_ffi.canonicalConfigMaxBytes
import uniffi.vpnhide_core_ffi.parseCanonicalImport
import java.io.InputStream

/** Shared Rust codec and self-target policy; persistence uses the ordinary Core command. */
internal fun parseImportedCanonicalConfig(
    raw: String,
    selfPkg: String,
): CanonicalConfig? = parseCanonicalImport(raw, selfPkg)?.toCanonical()

/** SAF byte adapter: use the shared admission limit before allocating an unbounded String. */
internal fun readCanonicalImportText(input: InputStream): String? {
    val limit = canonicalConfigMaxBytes().toInt()
    val bytes = ByteArray(limit + 1)
    var size = 0
    while (size < bytes.size) {
        val count = input.read(bytes, size, bytes.size - size)
        if (count < 0) break
        if (count == 0) return null
        size += count
    }
    return if (size <= limit) String(bytes, 0, size, Charsets.UTF_8) else null
}
