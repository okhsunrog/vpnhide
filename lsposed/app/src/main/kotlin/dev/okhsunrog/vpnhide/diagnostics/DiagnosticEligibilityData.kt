package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ObservationState

internal enum class RestartRequirement { None, App, Device }

internal enum class ConfigReadiness { Settled, Applying, Unknown, Failed }

internal enum class SelfRouting { VpnOff, Excluded, Routed }

internal enum class DiagnosticEligibility {
    Initializing,
    RestartApp,
    RestartDevice,
    Applying,
    ApplicationUnknown,
    ApplicationFailed,
    Checking,
    Unknown,
    VpnOff,
    SelfExcluded,
    Eligible,
}

/** Shared decision for screens and API; a previous value is not the result of a failed current attempt. */
internal fun diagnosticEligibility(
    initialized: Boolean,
    restart: RestartRequirement,
    config: ConfigReadiness,
    routing: ObservationState<SelfRouting>,
): DiagnosticEligibility =
    when {
        !initialized -> DiagnosticEligibility.Initializing
        restart == RestartRequirement.Device -> DiagnosticEligibility.RestartDevice
        restart == RestartRequirement.App -> DiagnosticEligibility.RestartApp
        config == ConfigReadiness.Applying -> DiagnosticEligibility.Applying
        config == ConfigReadiness.Unknown -> DiagnosticEligibility.ApplicationUnknown
        config == ConfigReadiness.Failed -> DiagnosticEligibility.ApplicationFailed
        routing.active != null -> DiagnosticEligibility.Checking
        routing.quarantined || routing.error != null -> DiagnosticEligibility.Unknown
        routing.lastGood == null -> if (routing.attempted) DiagnosticEligibility.Unknown else DiagnosticEligibility.Checking
        routing.lastGood.request.generation != routing.generation -> DiagnosticEligibility.Unknown
        routing.lastGood.value == SelfRouting.VpnOff -> DiagnosticEligibility.VpnOff
        routing.lastGood.value == SelfRouting.Excluded -> DiagnosticEligibility.SelfExcluded
        else -> DiagnosticEligibility.Eligible
    }
