package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.ConfigCoordinatorView
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.currentObservationValue

/** Inputs accepted by the domain, never a consumer's collection of independent flow values. */
internal data class DiagnosticSnapshotSources(
    val runs: DiagnosticRunView = DiagnosticRunView(),
    val routing: ObservationState<AppVpnStateSnapshot> = ObservationState(),
    val root: ObservationState<RootSnapshot> = ObservationState(),
    val config: ConfigCoordinatorView = ConfigCoordinatorView(),
    val inputs: DiagnosticInputs? = null,
    val impact: DiagnosticImpactState = DiagnosticImpactState(),
    val claimed: MeasurementKey? = null,
)

/**
 * One domain publication for UI, bridge and capture. [observedAt] uses the
 * monotonic observation clock. This is a revision of accepted knowledge, not a
 * claim that separate device observations were physically simultaneous.
 */
internal data class DiagnosticSnapshot(
    val revision: Long,
    val observedAt: Long,
    val sources: DiagnosticSnapshotSources,
    val presentation: DiagnosticPresentation,
    val situation: Situation,
) {
    val rootSnapshot: RootSnapshot? get() = sources.root.lastGood?.value
}

/** The only assembly path, including the context and time-dependent Situation. */
internal fun diagnosticSnapshot(
    sources: DiagnosticSnapshotSources,
    processIdentity: String,
    revision: Long,
    now: Long,
    wallTime: Long,
): DiagnosticSnapshot {
    val gate = sources.routing.gateProjection()
    val current = sources.inputs
    val observation =
        if (gate.attempted) {
            buildDiagnosticContextObservation(
                selfNeedsRestart = current?.selfNeedsRestart ?: false,
                appVpn =
                    AppVpnContext(
                        gate,
                        sources.routing.lastGood
                            ?.value
                            ?.identity,
                    ),
                snapshot = currentObservationValue(sources.root),
                config = sources.config.confirmed,
                selfPackage = current?.selfPackage.orEmpty(),
                processIdentity = processIdentity,
                now = wallTime,
                readiness = coreMeasurementReadiness(sources.config, configReadiness(sources.impact)),
                changeEpoch = sources.impact.changeEpoch,
                initialized = current != null,
                delivery = desiredDeliveryIdentity(sources.config, current?.selfPackage.orEmpty()),
                configJoined = coreInputsJoined(currentObservationValue(sources.root), sources.config),
            )
        } else {
            null
        }
    val knowledge = routingKnowledge(selfRoutingObservation(gate), now)
    val presentation = diagnosticPresentation(sources.runs, observation, sources.impact.changeEpoch, knowledge, sources.claimed)
    return DiagnosticSnapshot(revision, now, sources, presentation, situation(presentation, now))
}

/** A newer accepted root is valid too; waiting for an exact superseded ID could never finish. */
internal fun DiagnosticSnapshot.includesRoot(root: RootSnapshot): Boolean =
    rootSnapshot?.let { it.generation >= root.generation && it.observationId >= root.observationId } == true
