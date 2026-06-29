package dev.okhsunrog.vpnhide

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerializationException
import kotlinx.serialization.encodeToString
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.SecureRandom
import java.util.Base64
import kotlin.concurrent.thread

private const val TAG = "VpnHideAgentBridge"
private const val LOCALHOST = "127.0.0.1"

internal object AgentControlBridge {
    private val lock = Any()

    @Volatile private var server: BridgeServer? = null

    suspend fun setEnabled(
        context: Context,
        enabled: Boolean,
    ) {
        withContext(Dispatchers.IO) {
            synchronized(lock) {
                if (enabled) {
                    if (server == null) {
                        server = startServer(context.applicationContext)
                    }
                } else {
                    server?.stop()
                    server = null
                    File(context.filesDir, AGENT_BRIDGE_TOKEN_FILE).delete()
                }
            }
        }
    }

    private fun startServer(context: Context): BridgeServer? {
        return runCatching {
            val token = generateToken()
            val tokenFile = File(context.filesDir, AGENT_BRIDGE_TOKEN_FILE)
            tokenFile.writeText(token)
            BridgeServer(context.applicationContext, token).also { server ->
                server.start()
                Log.i(
                    TAG,
                    "Agent bridge listening on $LOCALHOST:$AGENT_BRIDGE_PORT token=$token tokenFile=${tokenFile.absolutePath}",
                )
            }
        }.onFailure { error ->
            Log.e(TAG, "Failed to start agent bridge", error)
        }.getOrNull()
    }

    private fun generateToken(): String {
        val bytes = ByteArray(32)
        SecureRandom().nextBytes(bytes)
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)
    }
}

private class BridgeServer(
    private val context: Context,
    private val token: String,
) {
    private val socket =
        ServerSocket().apply {
            reuseAddress = true
            bind(InetSocketAddress(InetAddress.getByName(LOCALHOST), AGENT_BRIDGE_PORT))
        }

    @Volatile private var running = true
    private lateinit var worker: Thread

    fun start() {
        worker =
            thread(name = "VpnHideAgentBridge", isDaemon = true) {
                serveLoop()
            }
    }

    fun stop() {
        running = false
        runCatching { socket.close() }
        if (::worker.isInitialized) worker.interrupt()
    }

    private fun serveLoop() {
        while (running && !socket.isClosed) {
            val client =
                try {
                    socket.accept()
                } catch (_: IOException) {
                    if (running) Log.w(TAG, "Agent bridge accept failed")
                    continue
                }
            client.use(::handleClient)
        }
    }

    private fun handleClient(client: Socket) {
        try {
            val request = readHttpRequest(client.getInputStream())
            if (request == null) {
                writeError(client, 400, "Bad Request", "Invalid HTTP request")
                return
            }
            if (request.headers["authorization"] != "Bearer $token") {
                writeError(client, 401, "Unauthorized", "Missing or invalid bearer token")
                return
            }
            when {
                request.method == "GET" && request.path == "/functions" -> {
                    writeJson(client, 200, "OK", AgentControlDispatcher.functionsJson())
                }

                request.method == "POST" && request.path == "/call" -> {
                    val result =
                        runBlocking {
                            AgentControlDispatcher.call(context, request.body)
                        }
                    writeJson(client, 200, "OK", result)
                }

                else -> {
                    writeError(client, 404, "Not Found", "Unknown endpoint")
                }
            }
        } catch (e: IllegalArgumentException) {
            writeError(client, 400, "Bad Request", e.message ?: "Invalid request")
        } catch (e: SerializationException) {
            writeError(client, 400, "Bad Request", e.message ?: "Invalid JSON")
        } catch (e: Throwable) {
            Log.e(TAG, "Agent bridge request failed", e)
            writeError(client, 500, "Internal Server Error", e.message ?: e::class.java.simpleName)
        }
    }
}

private data class HttpRequest(
    val method: String,
    val path: String,
    val headers: Map<String, String>,
    val body: String,
)

private fun readHttpRequest(input: InputStream): HttpRequest? {
    val requestLine = readAsciiLine(input)?.takeIf { it.isNotBlank() } ?: return null
    val requestParts = requestLine.split(' ', limit = 3)
    if (requestParts.size < 2) return null

    val headers = linkedMapOf<String, String>()
    while (true) {
        val line = readAsciiLine(input) ?: return null
        if (line.isEmpty()) break
        val separator = line.indexOf(':')
        if (separator <= 0) continue
        headers[line.substring(0, separator).trim().lowercase()] = line.substring(separator + 1).trim()
    }

    val contentLength = headers["content-length"]?.toIntOrNull() ?: 0
    val body = if (contentLength > 0) readBody(input, contentLength).decodeToString() else ""
    return HttpRequest(method = requestParts[0], path = requestParts[1], headers = headers, body = body)
}

private fun readAsciiLine(input: InputStream): String? {
    val out = ByteArrayOutputStream()
    while (true) {
        val next = input.read()
        if (next == -1) return if (out.size() == 0) null else out.toString(Charsets.US_ASCII.name())
        if (next == '\n'.code) break
        if (next != '\r'.code) out.write(next)
    }
    return out.toString(Charsets.US_ASCII.name())
}

private fun readBody(
    input: InputStream,
    length: Int,
): ByteArray {
    val body = ByteArray(length)
    var offset = 0
    while (offset < length) {
        val read = input.read(body, offset, length - offset)
        if (read == -1) throw IOException("Unexpected end of request body")
        offset += read
    }
    return body
}

private fun writeError(
    client: Socket,
    status: Int,
    reason: String,
    error: String,
) {
    writeJson(client, status, reason, AgentBridgeJson.encodeToString(AgentBridgeError(error)))
}

private fun writeJson(
    client: Socket,
    status: Int,
    reason: String,
    body: String,
) {
    val bytes = body.toByteArray(Charsets.UTF_8)
    val header =
        "HTTP/1.1 $status $reason\r\n" +
            "Content-Type: application/json; charset=utf-8\r\n" +
            "Content-Length: ${bytes.size}\r\n" +
            "Connection: close\r\n" +
            "\r\n"
    val output = client.getOutputStream()
    output.write(header.toByteArray(Charsets.US_ASCII))
    output.write(bytes)
    output.flush()
}
