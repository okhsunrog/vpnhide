package dev.okhsunrog.vpnhide

internal fun transitionFailureCode(failure: TransitionFailure): String = failure.name.replace(Regex("([a-z])([A-Z])"), "$1_$2").lowercase()

internal fun CanonicalWriteResult.toAgentMutationResult(restartTargets: Boolean = false): AgentMutationResult {
    val result = operation
    val changed = result?.phases?.get(ConfigPhase.Persist) == PhaseOutcome.Confirmed
    val failure = result?.failure?.let(::transitionFailureCode) ?: if (succeeded) null else output.ifBlank { "unavailable" }
    return AgentMutationResult(
        ok = succeeded,
        changed = changed,
        targetRestartRecommended = restartTargets && changed,
        message =
            when {
                failure == "ui_edit_conflict" -> {
                    "The operation overlaps unsaved UI edits. " +
                        "UI edits remain pending; see conflicts and phases for what already happened."
                }

                succeeded -> {
                    "Requested configuration operation completed."
                }

                else -> {
                    "Configuration operation failed: $failure. See phases for confirmed and unattempted work."
                }
            },
        errorCode = failure,
        phases =
            result
                ?.phases
                ?.mapKeys {
                    it.key.name.lowercase()
                }?.mapValues {
                    it.value.name
                        .replace(Regex("([a-z])([A-Z])"), "$1_$2")
                        .lowercase()
                }.orEmpty(),
        conflicts = result?.conflicts?.map { it.segments }.orEmpty(),
        draftPending = result?.draftPending == true,
    )
}
