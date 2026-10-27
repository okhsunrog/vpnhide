package dev.okhsunrog.vpnhide

/** Evidence reported by core; the app never dispatches these phases. */
internal enum class ConfigPhase { Persist, Secret, Cleanup, Native, Ports }

internal enum class PhaseOutcome { NotAttempted, Running, Confirmed, FailedKnown, Unknown }

internal enum class OperationSource { Ui, Bridge, System }

internal data class ConfigOperationSpec(
    val source: OperationSource,
    val writes: Set<ConfigField>,
    val protectsDrafts: Boolean = false,
    val removesCanonical: Boolean = false,
)

internal data class ConfigOperationResult(
    val id: Long,
    val phases: Map<ConfigPhase, PhaseOutcome>,
    val failure: TransitionFailure? = null,
    val conflicts: Set<ConfigField> = emptySet(),
    val draftPending: Boolean = false,
)

/** App intent lifecycle for diagnostics, independent of core's execution/recovery machine. */
internal interface ConfigOperationObserver {
    fun accepted(
        id: Long,
        spec: ConfigOperationSpec,
    )

    fun prepared(
        id: Long,
        spec: ConfigOperationSpec,
    )

    fun dispatched(
        id: Long,
        phase: ConfigPhase,
    )

    fun settled(result: ConfigOperationResult)

    fun recovered(result: ConfigOperationResult)

    object None : ConfigOperationObserver {
        override fun accepted(
            id: Long,
            spec: ConfigOperationSpec,
        ) = Unit

        override fun prepared(
            id: Long,
            spec: ConfigOperationSpec,
        ) = Unit

        override fun dispatched(
            id: Long,
            phase: ConfigPhase,
        ) = Unit

        override fun settled(result: ConfigOperationResult) = Unit

        override fun recovered(result: ConfigOperationResult) = Unit
    }
}
