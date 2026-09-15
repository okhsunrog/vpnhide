package dev.okhsunrog.vpnhide

internal enum class ConfigStatusMessage {
    Initializing,
    RebootRequired,
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

internal fun configStatus(view: ConfigCoordinatorView): ConfigStatus? {
    if (view.rechecking) return ConfigStatus(ConfigStatusMessage.Rechecking)
    val modeMessage =
        when (view.mode) {
            ConfigCoordinatorMode.Initializing -> ConfigStatusMessage.Initializing
            ConfigCoordinatorMode.RebootRequired -> ConfigStatusMessage.RebootRequired
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
    val result = view.lastResult ?: return null
    if (result.failure == null) return null
    val failed = setOf(PhaseOutcome.FailedKnown, PhaseOutcome.Unknown)
    val message =
        when {
            result.failure == TransitionFailure.UiEditConflict -> ConfigStatusMessage.Conflict
            result.phases[ConfigPhase.Secret] in failed -> ConfigStatusMessage.SecretFailed
            result.phases[ConfigPhase.Cleanup] in failed -> ConfigStatusMessage.CleanupFailed
            result.phases[ConfigPhase.Persist] == PhaseOutcome.Confirmed -> ConfigStatusMessage.ApplyFailed
            else -> ConfigStatusMessage.WriteFailed
        }
    return ConfigStatus(message, canApply = message == ConfigStatusMessage.ApplyFailed)
}
