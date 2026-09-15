package dev.okhsunrog.vpnhide

internal enum class ConfigStatusMessage {
    Initializing,
    Paused,
    Unavailable,
    Invalid,
    Missing,
    Conflict,
    SecretFailed,
    CleanupFailed,
    ApplyFailed,
    WriteFailed,
    Applying,
    Rechecking,
}

internal data class ConfigStatus(
    val message: ConfigStatusMessage,
    val canRecheck: Boolean = false,
    val canApply: Boolean = false,
)

internal data class ConfigFailureNotice(
    val operationId: Long,
    val status: ConfigStatus,
)

internal data class ConfigDialogState(
    val visible: Boolean = false,
    val dismissedMode: ConfigCoordinatorMode? = null,
)

internal sealed interface ConfigDialogEvent {
    data class Observed(
        val mode: ConfigCoordinatorMode,
        val rechecking: Boolean = false,
    ) : ConfigDialogEvent

    data class WriteAttempted(
        val mode: ConfigCoordinatorMode,
    ) : ConfigDialogEvent

    data class Dismissed(
        val mode: ConfigCoordinatorMode,
    ) : ConfigDialogEvent
}

internal fun reduceConfigDialog(
    state: ConfigDialogState,
    event: ConfigDialogEvent,
): ConfigDialogState =
    when (event) {
        is ConfigDialogEvent.Dismissed -> {
            ConfigDialogState(dismissedMode = event.mode)
        }

        is ConfigDialogEvent.WriteAttempted -> {
            state.copy(visible = event.mode != ConfigCoordinatorMode.Open)
        }

        is ConfigDialogEvent.Observed -> {
            when {
                event.mode == ConfigCoordinatorMode.Open -> ConfigDialogState()
                configNeedsAttention(event.mode) && !event.rechecking && state.dismissedMode != event.mode -> state.copy(visible = true)
                else -> state
            }
        }
    }

/** Only exceptional admission failures interrupt browsing with a dialog. */
internal fun configNeedsAttention(mode: ConfigCoordinatorMode): Boolean =
    mode in
        setOf(
            ConfigCoordinatorMode.Paused,
            ConfigCoordinatorMode.Unavailable,
            ConfigCoordinatorMode.Invalid,
        )

/** Routine progress stays at the initiating control, and success needs no notice. */
internal fun configFailureNotice(view: ConfigCoordinatorView): ConfigFailureNotice? {
    if (view.mode != ConfigCoordinatorMode.Open || view.rechecking || view.operations.isNotEmpty()) return null
    val result = view.lastResult ?: return null
    if (result.failure == null) return null
    return configStatus(view)?.let { ConfigFailureNotice(result.id, it) }
}

internal fun configStatus(view: ConfigCoordinatorView): ConfigStatus? {
    if (view.rechecking) return ConfigStatus(ConfigStatusMessage.Rechecking)
    val modeMessage =
        when (view.mode) {
            ConfigCoordinatorMode.Initializing -> ConfigStatusMessage.Initializing
            ConfigCoordinatorMode.Paused -> ConfigStatusMessage.Paused
            ConfigCoordinatorMode.Unavailable -> ConfigStatusMessage.Unavailable
            ConfigCoordinatorMode.Invalid -> ConfigStatusMessage.Invalid
            ConfigCoordinatorMode.Missing -> ConfigStatusMessage.Missing
            ConfigCoordinatorMode.Open -> null
        }
    if (modeMessage !=
        null
    ) {
        return ConfigStatus(
            modeMessage,
            canRecheck =
                view.mode !in setOf(ConfigCoordinatorMode.Initializing, ConfigCoordinatorMode.Missing),
        )
    }
    if (view.operations.isNotEmpty()) return ConfigStatus(ConfigStatusMessage.Applying)
    return view.lastResult?.let(::configFailureStatus)
}

private fun configFailureStatus(result: ConfigOperationResult): ConfigStatus? {
    if (result.failure == null) return null
    val failed = setOf(PhaseOutcome.FailedKnown, PhaseOutcome.Unknown)
    val message =
        when {
            result.failure == TransitionFailure.UiEditConflict -> ConfigStatusMessage.Conflict
            result.phases[ConfigPhase.Secret] in failed -> ConfigStatusMessage.SecretFailed
            result.phases[ConfigPhase.Cleanup] in failed -> ConfigStatusMessage.CleanupFailed
            result.phases[ConfigPhase.Native] in failed || result.phases[ConfigPhase.Ports] in failed -> ConfigStatusMessage.ApplyFailed
            result.phases[ConfigPhase.Persist] == PhaseOutcome.Confirmed -> ConfigStatusMessage.ApplyFailed
            else -> ConfigStatusMessage.WriteFailed
        }
    return ConfigStatus(message, canApply = message == ConfigStatusMessage.ApplyFailed)
}
