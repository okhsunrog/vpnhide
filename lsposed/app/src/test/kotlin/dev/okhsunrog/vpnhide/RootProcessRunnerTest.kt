package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.TimeUnit

class RootProcessRunnerTest {
    @Test
    fun `input travels separately from arguments and both output pipes drain`() {
        val runner = RootProcessRunner()
        val result = runner.run(listOf("sh", "-c", "cat; printf ignored >&2"), "private input".toByteArray())
        assertEquals(RootProcessResult.Completed("private input"), result)
    }

    @Test
    fun `nonzero exit missing executable and truncated reply stay uncertain`() {
        val runner = RootProcessRunner()
        assertEquals(RootProcessResult.Uncertain, runner.run(listOf("sh", "-c", "exit 1")))
        assertEquals(RootProcessResult.Uncertain, runner.run(listOf("/missing-vpnhide-test-executable")))
        assertEquals(RootProcessResult.Uncertain, runner.run(listOf("sh", "-c", "printf toolong"), outputLimit = 3))
    }

    @Test
    fun `blocked stdin and inherited pipes cannot extend caller deadline`() {
        val started = System.nanoTime()
        val result = RootProcessRunner().run(listOf("sh", "-c", "sleep 2"), ByteArray(1024 * 1024), timeoutMillis = 100)
        assertEquals(RootProcessResult.Uncertain, result)
        assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started) < 1_500)
    }
}
