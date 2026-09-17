package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.ReadReason
import dev.okhsunrog.vpnhide.TransitionFailure
import kotlinx.serialization.Serializable

internal enum class RestartRequirement { None, App, Device }

internal enum class ConfigReadiness { Settled, Applying, Unknown, Failed }

@Serializable
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

        // Invalidated without a failure: a re-read is owed (the VPN watcher's
        // debounce, a config write's refresh). That is a check in progress, not an
        // inability to determine; Unknown would flash "couldn't determine" on
        // every VPN toggle for the 750 ms before the read starts.
        routing.lastGood.request.generation != routing.generation -> DiagnosticEligibility.Checking

        routing.lastGood.value == SelfRouting.VpnOff -> DiagnosticEligibility.VpnOff

        routing.lastGood.value == SelfRouting.Excluded -> DiagnosticEligibility.SelfExcluded

        else -> DiagnosticEligibility.Eligible
    }

/** What is known about this app's routing right now: the fact, a re-read in flight with the last fact kept, or no usable fact. */
internal sealed interface RoutingKnowledge {
    data class Known(
        val routing: SelfRouting,
        val observedAt: Long,
    ) : RoutingKnowledge

    /** A re-read is owed or running; [lastKnown] is the fact before it (null when never read). */
    data class Verifying(
        val lastKnown: SelfRouting?,
        val reason: ReadReason,
        val since: Long,
    ) : RoutingKnowledge

    /** The read failed, the source is quarantined, or an attempt produced no value; [lastKnown] is history only. */
    data class Unknown(
        val cause: TransitionFailure?,
        val lastKnown: SelfRouting?,
    ) : RoutingKnowledge
}

/**
 * The routing observation as knowledge rather than as an admission decision.
 *
 * The branches are exactly [diagnosticEligibility]'s routing branches, in the
 * same order, so `Verifying` ⇔ `Checking`, `Unknown` ⇔ `Unknown` and `Known` ⇔
 * VpnOff / SelfExcluded / Eligible for any state once the process is initialized,
 * no restart is pending and the config is settled. What it adds is what
 * eligibility throws away: the last fact through a re-read, why that re-read
 * runs and since when — measured on the coordinator's monotonic base
 * ([dev.okhsunrog.vpnhide.ObservationClock]), which [now] must share.
 */
internal fun routingKnowledge(
    state: ObservationState<SelfRouting>,
    now: Long,
): RoutingKnowledge {
    val stale = state.stale
    val lastGood = state.lastGood
    val active = state.active
    return when {
        active != null -> {
            RoutingKnowledge.Verifying(
                lastKnown = lastGood?.value,
                reason = maxOf(active.reason, stale?.reason ?: active.reason),
                since = stale?.since ?: active.startedAt,
            )
        }

        state.quarantined || state.error != null -> {
            RoutingKnowledge.Unknown(state.error ?: TransitionFailure.ResourceUnavailable, lastGood?.value)
        }

        // Never read: an attempt that produced nothing is a real inability; no
        // attempt yet is the first read still owed.
        lastGood == null -> {
            if (state.attempted) {
                RoutingKnowledge.Unknown(null, null)
            } else {
                RoutingKnowledge.Verifying(null, stale?.reason ?: ReadReason.Background, stale?.since ?: now)
            }
        }

        lastGood.request.generation != state.generation -> {
            RoutingKnowledge.Verifying(lastGood.value, stale?.reason ?: ReadReason.Background, stale?.since ?: now)
        }

        else -> {
            RoutingKnowledge.Known(lastGood.value, lastGood.finishedAt)
        }
    }
}
