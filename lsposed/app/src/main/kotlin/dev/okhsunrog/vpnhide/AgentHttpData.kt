package dev.okhsunrog.vpnhide

import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.SocketTimeoutException

internal data class AgentRequestHead(
    val method: String,
    val path: String,
    val headers: Map<String, String>,
)

/** Bounds receiving one request. The deadline ends before dispatching an accepted operation. */
internal class AgentRequestReader(
    input: InputStream,
    private val setTimeout: (Int) -> Unit,
    private val nanoTime: () -> Long = System::nanoTime,
) {
    private val input = BufferedInputStream(input)
    private val startedAt = nanoTime()
    private var headBytes = 0

    fun readHead(): AgentRequestHead? {
        val request = readLine()?.takeIf { it.isNotBlank() } ?: return null
        val parts = request.split(' ', limit = 3)
        if (parts.size < 2) return null
        val headers = linkedMapOf<String, String>()
        var count = 0
        while (true) {
            val line = readLine() ?: return null
            if (line.isEmpty()) break
            require(++count <= MAX_HEADERS) { "Too many request headers" }
            val separator = line.indexOf(':')
            if (separator <= 0) continue
            headers[line.substring(0, separator).trim().lowercase()] = line.substring(separator + 1).trim()
        }
        checkDeadline()
        return AgentRequestHead(parts[0], parts[1], headers)
    }

    fun readBody(headers: Map<String, String>): String {
        val length = headers["content-length"]?.toIntOrNull() ?: 0
        if (length <= 0) return ""
        require(length <= MAX_BODY_BYTES) { "Request body too large" }
        val body = ByteArray(length)
        var offset = 0
        while (offset < length) {
            checkDeadline()
            val count = input.read(body, offset, length - offset)
            if (count == -1) throw IOException("Unexpected end of request body")
            offset += count
        }
        checkDeadline()
        return body.decodeToString()
    }

    private fun readLine(): String? {
        val output = ByteArrayOutputStream()
        while (true) {
            checkDeadline()
            val next = input.read()
            if (next == -1) return null
            require(++headBytes <= MAX_HEAD_BYTES) { "Request headers too large" }
            if (next == '\n'.code) break
            if (next != '\r'.code) {
                require(output.size() < MAX_LINE_BYTES) { "Request header line too long" }
                output.write(next)
            }
        }
        return output.toString(Charsets.US_ASCII.name())
    }

    private fun checkDeadline() {
        val remaining = REQUEST_TIMEOUT_NANOS - (nanoTime() - startedAt)
        if (remaining <= 0) throw SocketTimeoutException("Request receive deadline exceeded")
        setTimeout(((remaining + NANOS_PER_MILLI - 1) / NANOS_PER_MILLI).toInt())
    }

    private companion object {
        const val NANOS_PER_MILLI = 1_000_000L
        const val REQUEST_TIMEOUT_NANOS = 5_000 * NANOS_PER_MILLI
        const val MAX_HEAD_BYTES = 32 * 1024
        const val MAX_HEADERS = 64
        const val MAX_LINE_BYTES = 8 * 1024
        const val MAX_BODY_BYTES = 256 * 1024
    }
}
