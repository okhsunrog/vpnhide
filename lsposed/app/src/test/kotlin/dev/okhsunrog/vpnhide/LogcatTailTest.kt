package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.LogcatTail
import org.junit.Assert.assertEquals
import org.junit.Test

class LogcatTailTest {
    @Test
    fun `repeated reads since the last stamp are stitched without loss or duplicates`() {
        val tail = LogcatTail("10-19 12:00:00.000")
        val first =
            """
            --------- beginning of main
            10-19 11:59:59.999  1  1 I old: before start
            10-19 12:00:00.000  1  1 I a: start
            10-19 12:00:01.500  1  1 I a: one
            10-19 12:00:01.500  1  1 I a: two
            """.trimIndent()
        assertEquals(3, tail.accept(first).size)
        assertEquals("10-19 12:00:01.500", tail.since)
        // logcat -T includes lines at exactly `since`: the kept ones are skipped, a new one at the same stamp is not.
        val second =
            """
            10-19 12:00:01.500  1  1 I a: one
            10-19 12:00:01.500  1  1 I a: two
            10-19 12:00:01.500  1  1 I a: three
            10-19 12:00:02.000  1  1 I a: four
            """.trimIndent()
        assertEquals(listOf("three", "four"), tail.accept(second).map { it.substringAfterLast(": ") })
        assertEquals(emptyList<String>(), tail.accept(""))
    }
}
