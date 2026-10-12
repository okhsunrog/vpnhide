package dev.okhsunrog.vpnhide

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Process-only preview state, separate from every observation and configuration store. */
internal class AgentUiSession(
    initiallyEnabled: Boolean = true,
) {
    private var enabled = initiallyEnabled
    private val mutableState = MutableStateFlow(AgentUiState())
    val state = mutableState.asStateFlow()
    private var nextSessionId = 0L

    @Synchronized
    fun start(args: AgentUiStartArgs): AgentUiState {
        check(enabled) { "Agent Control is disabled" }
        validateAgentUiStart(args)
        mutableState.value =
            AgentUiState(
                sessionId = ++nextSessionId,
                scenario = args.scenario,
                revision = 1,
                helpOpen = args.scenario == "help",
                destination = args.destination,
            )
        return state.value
    }

    @Synchronized
    fun navigate(args: AgentUiNavigationArgs): AgentUiState {
        val current = requireSession(args.sessionId)
        validateAgentUiDestination(args.destination)
        mutableState.value =
            current.copy(
                revision = current.revision + 1,
                composedRevision = null,
                helpOpen = true,
                destination = args.destination,
                observedDestination = null,
                lastAction = null,
            )
        return state.value
    }

    @Synchronized
    fun back(sessionId: Long): AgentUiState {
        val current = requireSession(sessionId)
        if (!current.helpOpen || current.scenario == "help") return stop(sessionId)
        mutableState.value =
            current.copy(
                revision = current.revision + 1,
                composedRevision = null,
                helpOpen = false,
                destination = null,
                observedDestination = null,
            )
        return state.value
    }

    @Synchronized
    fun stop(sessionId: Long): AgentUiState {
        requireSession(sessionId)
        clear()
        return state.value
    }

    @Synchronized
    fun setEnabled(value: Boolean) {
        enabled = value
        if (!value) clear()
    }

    @Synchronized
    fun interact(
        sessionId: Long,
        revision: Long,
        operation: () -> Unit,
    ) {
        val current = state.value
        if (current.scenario != null && current.sessionId == sessionId && current.revision == revision) operation()
    }

    @Synchronized
    fun clear() {
        mutableState.value = AgentUiState()
    }

    /** Late composition/click callbacks from an older session must not alter a newer one. */
    @Synchronized
    fun observe(
        sessionId: Long,
        revision: Long,
        destination: String? = state.value.observedDestination,
        action: String? = null,
    ) {
        val current = state.value
        if (current.scenario == null || current.sessionId != sessionId || current.revision != revision) return
        mutableState.value =
            current.copy(
                composedRevision = revision,
                observedDestination = destination,
                lastAction = action ?: current.lastAction,
            )
    }

    private fun requireSession(sessionId: Long): AgentUiState =
        state.value.also {
            require(it.scenario != null && it.sessionId == sessionId) { "UI scenario session is no longer active" }
        }
}

internal val agentUiSession = AgentUiSession(initiallyEnabled = false)
