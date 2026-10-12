package dev.okhsunrog.vpnhide

import kotlinx.serialization.Serializable

@Serializable
internal data class AgentUiScenarioSpec(
    val id: String,
    val description: String,
)

@Serializable
internal data class AgentUiStartArgs(
    val scenario: String,
    val destination: String? = null,
)

@Serializable
internal data class AgentUiNavigationArgs(
    val sessionId: Long,
    val destination: String? = null,
)

@Serializable
internal data class AgentUiSessionArgs(
    val sessionId: Long,
)

@Serializable
internal data class AgentUiState(
    val sessionId: Long = 0,
    val scenario: String? = null,
    val revision: Long = 0,
    val composedRevision: Long? = null,
    val helpOpen: Boolean = false,
    val destination: String? = null,
    val observedDestination: String? = null,
    val lastAction: String? = null,
)
