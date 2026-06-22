package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class ShellCommandBuildersTest {
    @Test
    fun `package UID expression uses literal awk field comparison`() {
        val expr = buildPackageUidsExpression("com.example.app", "U")

        assertTrue(expr.contains("awk -v p=\"package:com.example.app\""))
        assertTrue(expr.contains("\$1 == p"))
        assertFalse(expr.contains("grep"))
    }

    @Test
    fun `UID resolver preserves multi-profile UID splitting`() {
        val cmd = buildUidResolverCommand(listOf("com.example.app"), "/tmp/out")

        assertTrue(cmd.contains("n = split(\$2, ids, \",\")"))
        assertTrue(cmd.contains("for (i = 1; i <= n; i++) print ids[i]"))
        assertTrue(cmd.contains("echo \"\$UIDS\" > /tmp/out"))
    }

    @Test
    fun `package UID expression does not treat dots as regex wildcards`() {
        val allPkgs =
            """
            package:comXexampleXapp uid:99999
            package:com.example.app uid:10123
            """.trimIndent()

        assertEquals("10123", runPackageUidExpression(allPkgs, "com.example.app"))
    }

    @Test
    fun `package UID expression expands comma-separated profile UIDs`() {
        val allPkgs = "package:com.example.app uid:10123,1010123"

        assertEquals("10123\n1010123", runPackageUidExpression(allPkgs, "com.example.app"))
    }

    @Test
    fun `package UID expression keeps repeated package lines`() {
        val allPkgs =
            """
            package:com.example.app uid:10123
            package:com.example.app uid:1010123
            """.trimIndent()

        assertEquals("10123\n1010123", runPackageUidExpression(allPkgs, "com.example.app"))
    }

    @Test
    fun `package UID expression returns empty output for unknown packages`() {
        val allPkgs = "package:com.other.app uid:10123"

        assertEquals("", runPackageUidExpression(allPkgs, "com.example.app"))
    }

    @Test
    fun `VPN interface helper matches expected prefixes`() {
        assertEquals(
            listOf("tun", "ppp", "tap", "wg", "ipsec", "xfrm", "utun", "l2tp", "gre"),
            VPN_INTERFACE_PREFIXES,
        )
        assertTrue(isVpnInterfaceName("tun0"))
        assertTrue(isVpnInterfaceName("l2tp0"))
        assertTrue(isVpnInterfaceName("gre1"))
        assertTrue(isVpnInterfaceName("myvpn0"))
        assertFalse(isVpnInterfaceName("wlan0"))
        assertFalse(isVpnInterfaceName(""))
    }

    @Test
    fun `native prefix lists stay in sync with Kotlin`() {
        val repoRoot = locateRepoRoot()
        val sources =
            listOf(
                repoRoot.resolve("zygisk/src/filter.rs"),
                repoRoot.resolve("lsposed/native/src/lib.rs"),
                repoRoot.resolve("kmod/vpnhide_kmod.c"),
            )

        sources.forEach { source ->
            val text = source.readText()
            VPN_INTERFACE_PREFIXES.forEach { prefix ->
                assertTrue("${source.path} is missing VPN prefix '$prefix'", text.contains("\"$prefix\""))
            }
        }
    }

    private fun locateRepoRoot(): File {
        var dir = File(".").canonicalFile
        while (true) {
            if (dir.resolve("VERSION").isFile && dir.resolve("lsposed").isDirectory) {
                return dir
            }
            dir = dir.parentFile ?: break
        }
        error("Could not locate repo root from ${File(".").canonicalPath}")
    }

    private fun runPackageUidExpression(
        allPkgs: String,
        packageName: String,
    ): String {
        val script =
            "ALL_PKGS=\$(cat <<'EOF'\n" +
                allPkgs +
                "\nEOF\n" +
                "); " +
                buildPackageUidsExpression(packageName, "U") +
                "; printf '%s' \"\$U\""
        val proc =
            ProcessBuilder("sh", "-c", script)
                .redirectErrorStream(true)
                .start()
        val out = proc.inputStream.bufferedReader().readText()
        val exitCode = proc.waitFor()
        assertEquals("shell exited with output: $out", 0, exitCode)
        return out
    }
}
