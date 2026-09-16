package dev.okhsunrog.vpnhide

import java.util.Base64

internal const val SYSTEM_DATA_FILE_CONTEXT = "u:object_r:system_data_file:s0"

internal fun shellQuote(value: String): String = "'" + value.replace("'", "'\"'\"'") + "'"

internal fun buildAtomicRootOnlyRawWriteCommand(
    path: String,
    content: String,
): String {
    val b64 = base64NoWrap(content)
    return listOf(
        "TMP=$path.tmp.\$\$",
        "echo '$b64' | base64 -d > \"\$TMP\"",
        "chmod 600 \"\$TMP\" 2>/dev/null",
        "chown root:root \"\$TMP\" 2>/dev/null",
        "mv \"\$TMP\" $path",
    ).joinToString(" && ")
}

internal fun buildAtomicSystemDataRawWriteCommand(
    path: String,
    content: String,
    mode: String,
): String {
    val b64 = base64NoWrap(content)
    return listOf(
        "TMP=$path.tmp.\$\$",
        "echo '$b64' | base64 -d > \"\$TMP\"",
        "chmod $mode \"\$TMP\" 2>/dev/null",
        "chown root:system \"\$TMP\" 2>/dev/null",
        "(chcon $SYSTEM_DATA_FILE_CONTEXT \"\$TMP\" 2>/dev/null || true)",
        "mv \"\$TMP\" $path",
    ).joinToString(" && ")
}

private fun base64NoWrap(content: String): String = Base64.getEncoder().encodeToString(content.toByteArray())
