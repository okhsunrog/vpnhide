package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.ConfigPhase
import dev.okhsunrog.vpnhide.PhaseOutcome

/** Desired policy and confirmed delivery are separate from the suite's actual observations. */
internal data class DesiredDeliveryIdentity(
    val desiredRevision: Long?,
    val identity: String,
    val confirmed: Boolean,
)

internal fun desiredDeliveryIdentity(
    config: ConfigCoordinatorView,
    selfPackage: String,
): DesiredDeliveryIdentity {
    val self = config.confirmed?.apps?.get(selfPackage)
    val required =
        buildSet {
            if (self?.native?.enabled == true) add(ConfigPhase.Native)
            if (self?.ports == true) add(ConfigPhase.Ports)
        }
    val evidence = config.applyEvidence.filter { it.phase in required }.sortedBy { it.phase.ordinal }
    val confirmed =
        config.coreCurrent && config.confirmed != null && (ConfigPhase.Native !in required || config.nativeObservationCurrent) &&
            config.coreOperations.isEmpty() && required.none { it in config.heldResources } &&
            required.all { phase ->
                evidence.any {
                    it.phase == phase && it.desiredRevision == config.desiredRevision &&
                        it.outcome == PhaseOutcome.Confirmed
                }
            }
    val identity = "required=$required;delivery=$evidence;held=${config.heldResources.intersect(
        required,
    )};running=${config.coreOperations.sorted()}"
    return DesiredDeliveryIdentity(config.desiredRevision, identity, confirmed)
}

/** Ongoing effects hold admission; retained uncertainty still permits honest actual measurements. */
internal fun coreMeasurementReadiness(
    config: ConfigCoordinatorView,
    local: ConfigReadiness,
): ConfigReadiness =
    if (config.coreOperations.isNotEmpty() ||
        local == ConfigReadiness.Applying
    ) {
        ConfigReadiness.Applying
    } else {
        ConfigReadiness.Settled
    }
