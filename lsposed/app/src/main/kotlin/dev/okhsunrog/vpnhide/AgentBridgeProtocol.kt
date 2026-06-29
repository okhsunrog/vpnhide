package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject

internal const val AGENT_BRIDGE_PORT = 27193
internal const val AGENT_BRIDGE_TOKEN_FILE = "agent_bridge_token"

internal val AgentBridgeJson =
    Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
    }

@Serializable
internal data class AgentBridgeCall(
    val fn: String,
    val args: JsonObject = JsonObject(emptyMap()),
)

@Serializable
internal data class AgentBridgeError(
    val error: String,
)

@Serializable
internal data class AgentBridgeFunctionSpec(
    val name: String,
    val description: String,
    val inputSchema: JsonObject,
)

@Serializable
internal data class AgentBridgeFunctionsResponse(
    val functions: List<AgentBridgeFunctionSpec>,
)
