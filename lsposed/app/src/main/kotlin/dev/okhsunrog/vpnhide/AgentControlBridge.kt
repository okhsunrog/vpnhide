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
import java.net.SocketTimeoutException
import java.security.SecureRandom
import java.util.Base64
import kotlin.concurrent.thread

private const val TAG = LogTags.AGENT_BRIDGE
private const val LOCALHOST = "127.0.0.1"

// A stalled localhost peer must not wedge the single serve thread forever:
// every accepted socket gets this read deadline before any byte is read.
private const val SOCKET_TIMEOUT_MS = 5_000

// Upper bound on a request body. The /call payload is a small JSON document;
// anything larger is rejected (after auth) rather than allocated.
private const val MAX_BODY_BYTES = 256 * 1024

// Pre-auth guard: cap a single request/header line so an unauthenticated peer
// cannot grow the line buffer without bound before we ever check the token.
private const val MAX_LINE_BYTES = 8 * 1024

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

    private fun startServer(context: Context): BridgeServer? =
        runCatching {
            val token = generateToken()
            val tokenFile = File(context.filesDir, AGENT_BRIDGE_TOKEN_FILE)
            tokenFile.writeText(token)
            BridgeServer(context.applicationContext, token).also { server ->
                server.start()
                // The token IS logged on purpose: tools/agent-mcp/server.py reads
                // it from logcat (`logcat -s VpnHideAgentBridge`) as the fallback
                // for retrieving it on RELEASE builds, where `run-as cat
                // files/agent_bridge_token` is unavailable. The bridge is an
                // opt-in, off-by-default, loopback-only debug feature, so emitting
                // the credential to logcat (readable only with READ_LOGS) is an
                // accepted trade-off for that workflow — do not gate or remove it.
                Log.i(
                    TAG,
                    "Agent bridge listening on $LOCALHOST:$AGENT_BRIDGE_PORT token=$token tokenFile=${tokenFile.absolutePath}",
                )
            }
        }.onFailure { error ->
            VpnHideLog.e(TAG, "Failed to start agent bridge", error)
        }.getOrNull()

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

    // The client currently being served, so stop() can unblock a worker parked
    // in a socket read (interrupting the thread does not abort blocking socket
    // I/O, and the ServerSocket close does not touch an accepted client socket).
    @Volatile private var activeClient: Socket? = null

    fun start() {
        worker =
            thread(name = LogTags.AGENT_BRIDGE, isDaemon = true) {
                serveLoop()
            }
    }

    fun stop() {
        running = false
        runCatching { socket.close() }
        runCatching { activeClient?.close() }
        if (::worker.isInitialized) worker.interrupt()
    }

    private fun serveLoop() {
        while (running && !socket.isClosed) {
            val client =
                try {
                    socket.accept().apply { soTimeout = SOCKET_TIMEOUT_MS }
                } catch (_: IOException) {
                    if (running) Log.w(TAG, "Agent bridge accept failed")
                    continue
                }
            activeClient = client
            client.use(::handleClient)
            activeClient = null
        }
    }

    private fun handleClient(client: Socket) {
        try {
            val input = client.getInputStream()
            val head = readRequestHead(input)
            if (head == null) {
                writeError(client, 400, "Bad Request", "Invalid HTTP request")
                return
            }
            // Authenticate on the headers BEFORE allocating/reading the body, so
            // an unauthenticated peer can never trigger a body-sized allocation.
            if (head.headers["authorization"] != "Bearer $token") {
                writeError(client, 401, "Unauthorized", "Missing or invalid bearer token")
                return
            }
            val body = readBody(input, head.headers)
            when {
                head.method == "GET" && head.path == "/functions" -> {
                    writeJson(client, 200, "OK", AgentControlDispatcher.functionsJson())
                }

                head.method == "POST" && head.path == "/call" -> {
                    val result =
                        runBlocking {
                            AgentControlDispatcher.call(context, body)
                        }
                    writeJson(client, 200, "OK", result)
                }

                else -> {
                    writeError(client, 404, "Not Found", "Unknown endpoint")
                }
            }
        } catch (_: SocketTimeoutException) {
            // Stalled/idle peer hit the read deadline; drop the connection quietly.
        } catch (e: IllegalArgumentException) {
            writeError(client, 400, "Bad Request", e.message ?: "Invalid request")
        } catch (e: SerializationException) {
            writeError(client, 400, "Bad Request", e.message ?: "Invalid JSON")
        } catch (e: Throwable) {
            VpnHideLog.e(TAG, "Agent bridge request failed", e)
            writeError(client, 500, "Internal Server Error", e.message ?: e::class.java.simpleName)
        }
    }
}

private data class RequestHead(
    val method: String,
    val path: String,
    val headers: Map<String, String>,
)

private fun readRequestHead(input: InputStream): RequestHead? {
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
    return RequestHead(method = requestParts[0], path = requestParts[1], headers = headers)
}

private fun readBody(
    input: InputStream,
    headers: Map<String, String>,
): String {
    val contentLength = headers["content-length"]?.toIntOrNull() ?: 0
    if (contentLength <= 0) return ""
    require(contentLength <= MAX_BODY_BYTES) { "Request body too large" }
    val body = ByteArray(contentLength)
    var offset = 0
    while (offset < contentLength) {
        val read = input.read(body, offset, contentLength - offset)
        if (read == -1) throw IOException("Unexpected end of request body")
        offset += read
    }
    return body.decodeToString()
}

private fun readAsciiLine(input: InputStream): String? {
    val out = ByteArrayOutputStream()
    while (true) {
        val next = input.read()
        if (next == -1) return if (out.size() == 0) null else out.toString(Charsets.US_ASCII.name())
        if (next == '\n'.code) break
        if (next != '\r'.code) {
            require(out.size() < MAX_LINE_BYTES) { "Request header line too long" }
            out.write(next)
        }
    }
    return out.toString(Charsets.US_ASCII.name())
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
