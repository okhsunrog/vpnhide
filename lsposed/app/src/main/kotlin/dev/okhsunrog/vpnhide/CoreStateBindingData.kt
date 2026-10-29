package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreConfigPhase
import uniffi.vpnhide_core_ffi.CoreObservationFreshness
import uniffi.vpnhide_core_ffi.CorePhaseOutcome
import uniffi.vpnhide_core_ffi.CorePublication
import uniffi.vpnhide_core_ffi.CoreSubmitRefusal
import uniffi.vpnhide_core_ffi.CoreOperationRecord as NativeOperationRecord

internal fun CorePublication.toState(): CoreStateView =
    CoreStateView(
        epoch = epoch.toLong(),
        configRevision = configRevision.toLong(),
        canonical =
            if (startupError != null) {
                RootCanonicalRead.Invalid
            } else {
                config?.let { runCatching { RootCanonicalRead.Available(it.toCanonical()) }.getOrDefault(RootCanonicalRead.Invalid) }
                    ?: RootCanonicalRead.Missing
            },
        operationsInFlight = operationsInFlight.map(ULong::toLong),
        debugLease = debugLease,
        revision = revision.toLong(),
        nativeBoot = backend.boot?.toBootStatus(),
        activeCaptures = debugLeases.size + if (debugLease && debugLeases.isEmpty()) 1 else 0,
        routing =
            CoreRoutingView(
                routing.freshness == CoreObservationFreshness.CURRENT,
                routing.generation.toLong(),
                routing.uid?.uid?.toInt(),
                routing.uid?.state,
                (routing.uid?.interfaces ?: routing.vpnInterfaces).distinct().sorted(),
                routing.uid?.session,
            ),
        mutationHeld = mutationHeld,
        recovery = recovery,
        startupError = startupError,
        storageFault = storageFault,
        applyEvidence =
            applyEvidence.map {
                CoreApplyIdentity(it.phase.toPhase(), it.desiredRevision.toLong(), it.op.toLong(), it.outcome.toOutcome())
            },
        heldResources = heldResources.mapTo(linkedSetOf()) { it.toPhase() },
        backend =
            CoreBackendView(
                backend.observedFreshness == CoreObservationFreshness.CURRENT,
                backend.selected?.toBackend(),
                backend.applied?.toBackend(),
                backend.active?.toBackend(),
                backend.installed.mapTo(linkedSetOf()) { it.toBackend() },
            ),
    )

internal fun NativeOperationRecord.toRecord(): CoreOperationRecord =
    CoreOperationRecord(op.toLong(), phases.associate { it.phase.toPhase() to it.outcome.toOutcome() }, settled?.toOutcome())

internal fun CoreConfigPhase.toPhase(): ConfigPhase =
    when (this) {
        CoreConfigPhase.PERSIST -> ConfigPhase.Persist
        CoreConfigPhase.SECRET -> ConfigPhase.Secret
        CoreConfigPhase.CLEANUP -> ConfigPhase.Cleanup
        CoreConfigPhase.NATIVE -> ConfigPhase.Native
        CoreConfigPhase.PORTS -> ConfigPhase.Ports
    }

internal fun CorePhaseOutcome.toOutcome(): PhaseOutcome =
    when (this) {
        CorePhaseOutcome.NOT_ATTEMPTED -> PhaseOutcome.NotAttempted
        CorePhaseOutcome.RUNNING -> PhaseOutcome.Running
        CorePhaseOutcome.CONFIRMED -> PhaseOutcome.Confirmed
        CorePhaseOutcome.FAILED_KNOWN -> PhaseOutcome.FailedKnown
        CorePhaseOutcome.UNKNOWN -> PhaseOutcome.Unknown
    }

internal fun CoreSubmitRefusal.toRefusal(): CoreReply.Refused = CoreReply.Refused(this)
