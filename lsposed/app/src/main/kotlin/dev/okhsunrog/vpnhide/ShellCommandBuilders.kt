package dev.okhsunrog.vpnhide

internal val VPN_INTERFACE_PREFIXES =
    listOf("tun", "ppp", "tap", "wg", "ipsec", "xfrm", "utun", "l2tp", "gre")

internal fun isVpnInterfaceName(name: String): Boolean {
    if (name.isEmpty()) return false
    val normalized = name.lowercase()
    return VPN_INTERFACE_PREFIXES.any { normalized.startsWith(it) } || normalized.contains("vpn")
}

internal fun buildPackageUidsExpression(
    packageName: String,
    outputVariable: String,
): String =
    "$outputVariable=\$(echo \"\$ALL_PKGS\" | awk -v p=\"package:$packageName\" " +
        "'\$1 == p { sub(/uid:/, \"\", \$2); n = split(\$2, ids, \",\"); " +
        "for (i = 1; i <= n; i++) print ids[i] }')"

internal fun buildUidResolverCommand(
    packages: List<String>,
    outputFile: String,
): String =
    buildString {
        append("ALL_PKGS=\"\$(pm list packages -U --user all 2>/dev/null)\"")
        append("; UIDS=\"\"")
        for (pkg in packages) {
            append("; ")
            append(buildPackageUidsExpression(pkg, "U"))
            append("; if [ -n \"\$U\" ]; then if [ -z \"\$UIDS\" ]; then UIDS=\"\$U\"; else UIDS=\"\$UIDS")
            append("\n")
            append("\$U\"; fi; fi")
        }
        append("; if [ -n \"\$UIDS\" ]; then echo \"\$UIDS\" > $outputFile 2>/dev/null")
        append("; else echo > $outputFile 2>/dev/null; fi")
    }
