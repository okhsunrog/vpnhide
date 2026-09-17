package dev.okhsunrog.vpnhide.diagnostics

import dev.okhsunrog.vpnhide.CanonicalConfig
import dev.okhsunrog.vpnhide.ObservationState
import dev.okhsunrog.vpnhide.ObservedValue
import dev.okhsunrog.vpnhide.RootSnapshot
import dev.okhsunrog.vpnhide.currentObservationValue
import dev.okhsunrog.vpnhide.detectNativeBackendStates
import dev.okhsunrog.vpnhide.displayNativeBackend
import dev.okhsunrog.vpnhide.lsposedHooksActiveThisBoot
import dev.okhsunrog.vpnhide.settings.installedNativeOptionalHooks
import java.util.concurrent.atomic.AtomicReference

/**
 * Result of one context observation effect: the shared eligibility decision plus
 * the comparable [MeasurementContext] when the routing observation is current.
 * Both are built by [buildDiagnosticContextObservation] from adapter-supplied
 * observations; nothing here reads a service or a shell.
 */
internal data class DiagnosticContextObservation(
    val eligibility: DiagnosticEligibility,
    val context: MeasurementContext?,
)

internal data class AppVpnContext(
    val routing: ObservationState<DiagnosticGate>,
    val identity: String?,
)

/**
 * The frozen vector set of one suite: every registered probe, keyed by stable id.
 * Ownership here is structural (the Java-implemented native-level probes have no
 * hook); the backend-scoped ownership of Rust probes is applied by the report
 * from the retained results, so an old measurement keeps its own coverage.
 */
internal fun diagnosticProbePlan(): List<ProbePlanEntry> =
    NATIVE_CHECKS.map { ProbePlanEntry(it.id) } +
        NATIVE_EXTRA_CHECKS.map { ProbePlanEntry(it.id, owned = false) } +
        (CORE_JAVA_CHECKS + EXTRA_JAVA_CHECKS).map { ProbePlanEntry(it.id) }

internal fun diagnosticRequest(automatic: Boolean = false): DiagnosticRequest =
    DiagnosticRequest(diagnosticProbePlan(), automatic = automatic)

/** Outcomes of planned probes only: an unplanned or unlabeled result cannot enter a run's evidence. */
internal fun plannedOutcomes(
    plan: List<ProbePlanEntry>,
    checks: List<CheckResult>,
): Map<String, CheckOutcome> {
    val planned = plan.mapTo(HashSet()) { it.id }
    return checks.filter { it.id in planned }.associate { it.id to it.outcome }
}

/** Core evidence is the base; the slow phase only contributes its own list. */
internal fun mergeDiagnosticEvidence(
    previous: CheckResults?,
    stage: DiagnosticStage,
    results: CheckResults,
): CheckResults =
    when (stage) {
        DiagnosticStage.Slow -> (previous ?: CheckResults(native = emptyList())).copy(extraJava = results.extraJava)
        else -> results
    }

/**
 * The routing gate cache observes network facts; a pending self-restart is process
 * readiness and is decided by [diagnosticEligibility] before routing is consulted,
 * so a NEEDS_RESTART value carries no routing fact at all.
 */
internal fun selfRoutingObservation(state: ObservationState<DiagnosticGate>): ObservationState<SelfRouting> =
    ObservationState(
        lastGood =
            state.lastGood?.let { observed ->
                observed.value.toSelfRouting()?.let { ObservedValue(it, observed.request, observed.finishedAt) }
            },
        active = state.active,
        generation = state.generation,
        stale = state.stale,
        nextId = state.nextId,
        error = state.error,
        attempted = state.attempted,
        attemptedGeneration = state.attemptedGeneration,
        quarantined = state.quarantined,
        quarantineRequestId = state.quarantineRequestId,
    )

private fun DiagnosticGate.toSelfRouting(): SelfRouting? =
    when (this) {
        DiagnosticGate.VPN_OFF -> SelfRouting.VpnOff
        DiagnosticGate.SELF_NOT_ROUTED -> SelfRouting.Excluded
        DiagnosticGate.ROUTED -> SelfRouting.Routed
        DiagnosticGate.NEEDS_RESTART -> null
    }

/** How a context effect obtains the routing observation it folds. */
internal enum class RoutingRead { Reuse, Join, Refresh }

/**
 * "Fresh" means not invalidated: a current observation is reused as is, an
 * in-flight read is joined, and only a stale, failed or absent observation forces
 * a new root snapshot plus routing probe. VPN callbacks and config writes
 * invalidate the observation, so a known change always causes a new read; this
 * keeps the end-context read of an undisturbed run free of a second root shell.
 */
internal fun <T> routingReadPlan(state: ObservationState<T>): RoutingRead =
    when {
        currentObservationValue(state) != null -> RoutingRead.Reuse
        state.active != null -> RoutingRead.Join
        else -> RoutingRead.Refresh
    }

/** The legacy blocking gate a NotStarted attempt renders as, or null when the reason has no gate vocabulary. */
internal fun DiagnosticEligibility.blockedGate(): DiagnosticGate? =
    when (this) {
        DiagnosticEligibility.RestartApp -> DiagnosticGate.NEEDS_RESTART
        DiagnosticEligibility.VpnOff -> DiagnosticGate.VPN_OFF
        DiagnosticEligibility.SelfExcluded -> DiagnosticGate.SELF_NOT_ROUTED
        else -> null
    }

/**
 * Fold the adapter's observations into eligibility plus the measurement context.
 *
 * [readiness] and [changeEpoch] come from the config-operation impact state
 * ([DiagnosticImpactState]): a relevant operation in flight makes the suite
 * Applying, an unresolved one ApplicationUnknown, and every mutating dispatch of
 * a relevant operation advances the epoch so an older measurement reads as
 * Changed. A config change is also caught by the self configuration identity.
 */
internal fun buildDiagnosticContextObservation(
    selfNeedsRestart: Boolean,
    appVpn: AppVpnContext,
    snapshot: RootSnapshot?,
    config: CanonicalConfig?,
    selfPackage: String,
    processIdentity: String,
    now: Long,
    readiness: ConfigReadiness = ConfigReadiness.Settled,
    changeEpoch: Long = 0,
    initialized: Boolean = true,
): DiagnosticContextObservation {
    val routing = appVpn.routing
    val restart = if (selfNeedsRestart) RestartRequirement.App else RestartRequirement.None
    val eligibility = diagnosticEligibility(initialized, restart, readiness, selfRoutingObservation(routing))
    val gate = currentObservationValue(routing)
    val context =
        if (gate != null && snapshot != null && appVpn.identity != null) {
            val coverage = measurementCoverageFor(snapshot)
            MeasurementContext(
                subject = "$processIdentity;boot:${snapshot.sections["current_boot_id"].orEmpty().trim()}",
                configuration = selfConfigurationIdentity(config, selfPackage),
                routing = appVpn.identity,
                coverage = coverage.identity,
                changeEpoch = changeEpoch,
                observationId = snapshot.observationId,
                observedAt = now,
                coverageLayers = coverage,
            )
        } else {
            null
        }
    return DiagnosticContextObservation(eligibility, context)
}

/**
 * The self UID's applied-config projection: its own roles and hook selection plus
 * the global optional features. Other apps' roles, debug logging and auto-hide
 * bookkeeping do not change what this process measures.
 */
internal fun selfConfigurationIdentity(
    config: CanonicalConfig?,
    selfPackage: String,
): String {
    if (config == null) return "config:unavailable"
    val self = config.apps[selfPackage]
    val role =
        self?.let {
            listOf(it.java, it.javaHooks, it.native.enabled, it.native.overrides, it.appHiding, it.ports, it.portPolicy, it.hidden)
        }
    return "self=$role;features=${config.settings.optionalFeatures.sorted()}"
}

/** The hiding layers a root snapshot shows: the active native backend, its installed optional hooks and LSPosed liveness this boot. */
internal fun measurementCoverage(sections: Map<String, String>): MeasurementCoverage {
    val bootId = sections["current_boot_id"].orEmpty().trim()
    val backend = displayNativeBackend(detectNativeBackendStates(sections, currentBootId = bootId))
    return MeasurementCoverage(
        backend = backend,
        installedOptionalHooks = installedNativeOptionalHooks(backend.id, sections, bootId),
        lsposedActive = lsposedHooksActiveThisBoot(sections["lsposed_state"].orEmpty(), bootId),
    )
}

/** Coverage identity string of a snapshot; see [MeasurementCoverage.identity]. */
internal fun coverageIdentity(sections: Map<String, String>): String = measurementCoverage(sections).identity

// The presentation projection re-derives the context on every emission of any of
// its sources, on every tab; backend detection over the snapshot sections is the
// only non-trivial part, and a snapshot's sections never change under one
// observation id, so the last derivation is kept.
private val coverageMemo = AtomicReference<Pair<Long, MeasurementCoverage>?>(null)

internal fun measurementCoverageFor(snapshot: RootSnapshot): MeasurementCoverage {
    coverageMemo.get()?.let { (id, coverage) -> if (id == snapshot.observationId) return coverage }
    return measurementCoverage(snapshot.sections).also { coverageMemo.set(snapshot.observationId to it) }
}
