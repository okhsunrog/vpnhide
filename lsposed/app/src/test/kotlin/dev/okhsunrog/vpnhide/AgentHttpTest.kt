package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.io.ByteArrayInputStream
import java.net.SocketTimeoutException

class AgentHttpTest {
    private fun reader(text: String): AgentRequestReader = AgentRequestReader(ByteArrayInputStream(text.toByteArray()), {})

    @Test
    fun `normal request remains usable after rejection`() {
        assertThrows(
            IllegalArgumentException::class.java,
        ) { reader("GET /functions HTTP/1.1\r\n" + "x: a\r\n".repeat(65) + "\r\n").readHead() }
        val request = reader("POST /call HTTP/1.1\r\nAuthorization: Bearer test\r\nContent-Length: 2\r\n\r\n{}")
        val head = requireNotNull(request.readHead())
        assertEquals("Bearer test", head.headers["authorization"])
        assertEquals("{}", request.readBody(head.headers))
    }

    @Test
    fun `total size includes ignored carriage returns and repeated headers`() {
        assertThrows(IllegalArgumentException::class.java) {
            reader("GET / HTTP/1.1\r\n" + "x: ${"a".repeat(700)}\r\n".repeat(60) + "\r\n").readHead()
        }
        assertThrows(IllegalArgumentException::class.java) { reader("GET / HTTP/1.1" + "\r".repeat(33 * 1024) + "\n\n").readHead() }
    }

    @Test
    fun `slow headers exhaust total deadline despite individual reads making progress`() {
        var now = 0L
        val request =
            AgentRequestReader(ByteArrayInputStream("GET / HTTP/1.1\r\n\r\n".toByteArray()), {}, { now.also { now += 400_000_000 } })
        assertThrows(SocketTimeoutException::class.java) { request.readHead() }
    }

    @Test
    fun `body shares deadline with headers`() {
        var now = 0L
        val request = AgentRequestReader(ByteArrayInputStream("POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}".toByteArray()), {}, { now })
        val head = requireNotNull(request.readHead())
        now = 5_000_000_000
        assertThrows(SocketTimeoutException::class.java) { request.readBody(head.headers) }
    }
}
