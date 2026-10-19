package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ShellCommandBuildersTest {
    @Test
    fun `superkey clear removes root only key file`() {
        val cmd = buildSuperkeyClearCommand()

        assertEquals("rm -f $SUPERKEY_FILE", cmd)
    }

    @Test
    fun `superkey write prepares tmp permissions before rename`() {
        val cmd = buildSuperkeyWriteCommand("keyy51311")

        assertTrue(cmd.startsWith("mkdir -p /data/adb/vpnhide && TMP=$SUPERKEY_FILE.tmp."))
        assertTrue(cmd.contains("base64 -d > \"\$TMP\" && chmod 600 \"\$TMP\" 2>/dev/null"))
        assertTrue(cmd.contains("&& chown root:root \"\$TMP\" 2>/dev/null && mv \"\$TMP\" $SUPERKEY_FILE"))
        assertTrue(!cmd.contains("mv \"\$TMP\" $SUPERKEY_FILE && chmod"))
    }
}
