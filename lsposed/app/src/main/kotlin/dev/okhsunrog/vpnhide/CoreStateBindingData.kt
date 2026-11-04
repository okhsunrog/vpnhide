package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreConfigPhase
import uniffi.vpnhide_core_ffi.CoreObservationFreshness
import uniffi.vpnhide_core_ffi.CorePhaseOutcome
import uniffi.vpnhide_core_ffi.CorePublication
import uniffi.vpnhide_core_ffi.CoreSubmitRefusal
import uniffi.vpnhide_core_ffi.CoreOperationRecord as NativeOperationRecord

internal fun CorePublication.toState(): CoreStateView =
    CoreStateView(
        attachmentEvidence = attachmentEvidence.copy(),
        maintenance = maintenance?.copy(),
        backendChoice = backendChoice?.copy(),
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
        nativeStatus = nativeView.toNativeStatusInfo(),
        mutationHeld = mutationHeld,
        autonomousDiagnostics = autonomousDiagnostics,
        autonomousAssessment = autonomousAssessment,
        zygiskLifecycle = zygiskLifecycle,
        zygiskRevocation = zygiskRevocation,
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
        CoreConfigPhase.MAINTENANCE -> ConfigPhase.Maintenance
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
