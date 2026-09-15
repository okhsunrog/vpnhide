package dev.okhsunrog.vpnhide

/** Pure transition output. The process owner publishes state before dispatching effects. */
internal data class Transition<S, E>(
    val state: S,
    val effects: List<E> = emptyList(),
)

/** Scoped to one machine instance; owners must also route completions to the correct machine. */
internal data class EffectTicket(
    val owner: Long,
    val sequence: Long,
)

/** Safe public failure categories. Root output and secrets do not belong in reducer state. */
internal enum class TransitionFailure {
    InitializationPending,
    ReadFailed,
    ValidationFailed,
    ExecutionFailed,
    DeadlineExceeded,
    ApplicationUnknown,
    ApplicationFailed,
    ContextChanged,
    ContextUnknown,
    ResourceUnavailable,
    Cancelled,
    Busy,
    UiEditConflict,
    MutationPaused,
}

/** Segments, not dotted strings: a package name containing dots is one segment. */
internal data class ConfigField(
    val segments: List<String>,
) {
    init {
        require(segments.isNotEmpty() && segments.none { it.isEmpty() })
    }
}

internal fun configFieldsOverlap(
    first: ConfigField,
    second: ConfigField,
): Boolean =
    first.segments.take(second.segments.size) == second.segments ||
        second.segments.take(first.segments.size) == first.segments

internal fun conflictingFields(
    writes: Set<ConfigField>,
    protected: Set<ConfigField>,
): Set<ConfigField> = writes.filterTo(linkedSetOf()) { field -> protected.any { configFieldsOverlap(field, it) } }
