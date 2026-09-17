package dev.okhsunrog.vpnhide.diagnostics

import android.content.Context
import android.os.Process
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.ConfigOperationObserver
import dev.okhsunrog.vpnhide.ConfigOperationResult
import dev.okhsunrog.vpnhide.ConfigOperationSpec
import dev.okhsunrog.vpnhide.ConfigPhase
import dev.okhsunrog.vpnhide.ContextObservationInputs
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.VpnHideLog
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * The production wiring of [DiagnosticDomain]: the process scope, the real probe
 * IO, and the shared observations (routing gate, root snapshot, confirmed config).
 * Callers supply a [Context] and the self-restart flag; the domain sees only the
 * pure [DiagnosticInputs] derived from them.
 */
internal object DiagnosticsCache {
    @Volatile private var inputs: ContextObservationInputs? = null

    // Whether this app's own hooks need a restart to apply (it was just added as a
    // target). Process-constant, so it is sticky-OR: once any caller reports true,
    // a caller that does not know it (the agent bridge) can safely pass false.
    @Volatile private var restartPending = false

    private val domainInputs = MutableStateFlow<DiagnosticInputs?>(null)

    private val domain: DiagnosticDomain by lazy {
        DiagnosticDomain(
            scope = ObservationRuntime.scope,
            io = AppDiagnosticRunIo(inputs = { inputs }, impact = { domain.impact }),
            routing = RoutingGateCache.observation,
            snapshots = RootSnapshotCache.snapshot,
            config = CanonicalConfigRepository.state,
            inputs = domainInputs,
            processIdentity = "pid:${Process.myPid()};uid:${Process.myUid()}",
            log = { VpnHideLog.i(LogTags.DIAG, it) },
        )
    }

    /** See [DiagnosticDomain.presentation]. */
    val presentation: StateFlow<DiagnosticPresentation> get() = domain.presentation

    /** See [DiagnosticDomain.situation]. */
    val situation: StateFlow<Situation> get() = domain.situation

    /** Startup intent: starts the first suite of the process; later calls join or read what exists. */
    fun run(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        domain.run()
    }

    /** Explicit re-check: always a new run. The one caller is `retryDiagnosticsAndDashboard`. */
    fun retry(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        updateInputs(context, selfNeedsRestart)
        domain.retry()
    }

    /** See [DiagnosticDomain.awaitTerminal]. */
    suspend fun awaitTerminal(
        context: Context,
        selfNeedsRestart: Boolean,
    ): DiagnosticPresentation {
        updateInputs(context, selfNeedsRestart)
        return domain.awaitTerminal()
    }

    /** See [DiagnosticDomain.captureRun]. */
    suspend fun captureRun(
        context: Context,
        selfNeedsRestart: Boolean,
        captureId: Long,
    ): DiagnosticCaptureOutcome {
        updateInputs(context, selfNeedsRestart)
        return domain.captureRun(captureId)
    }

    /** Config-operation lifecycle from the coordinator's observer. */
    fun configOperation(event: DiagnosticImpactEvent) = domain.configOperation(event)

    private fun updateInputs(
        context: Context,
        selfNeedsRestart: Boolean,
    ) {
        restartPending = restartPending || selfNeedsRestart
        val application = context.applicationContext
        inputs = ContextObservationInputs(application, restartPending)
        domainInputs.value = DiagnosticInputs(application.packageName, restartPending)
    }
}

/** The config coordinator's observer: classifies each operation against this app's own measurement. */
internal class DiagnosticImpactObserver(
    private val selfPackage: () -> String,
) : ConfigOperationObserver {
    override fun accepted(
        id: Long,
        spec: ConfigOperationSpec,
    ) = DiagnosticsCache.configOperation(DiagnosticImpactEvent.Accepted(id, operationAffectsSelfMeasurement(spec, selfPackage())))

    override fun prepared(
        id: Long,
        spec: ConfigOperationSpec,
    ) = DiagnosticsCache.configOperation(DiagnosticImpactEvent.Prepared(id, operationAffectsSelfMeasurement(spec, selfPackage())))

    override fun dispatched(
        id: Long,
        phase: ConfigPhase,
    ) = DiagnosticsCache.configOperation(DiagnosticImpactEvent.Dispatched(id, phase))

    override fun settled(result: ConfigOperationResult) =
        DiagnosticsCache.configOperation(DiagnosticImpactEvent.Settled(result.id, result.failure))

    override fun recovered(result: ConfigOperationResult) =
        DiagnosticsCache.configOperation(DiagnosticImpactEvent.Recovered(result.id, result.failure))
}
