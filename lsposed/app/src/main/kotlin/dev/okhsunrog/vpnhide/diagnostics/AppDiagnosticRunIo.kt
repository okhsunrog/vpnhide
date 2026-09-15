package dev.okhsunrog.vpnhide.diagnostics

import android.net.ConnectivityManager
import android.os.Process
import dev.okhsunrog.vpnhide.CanonicalConfigRepository
import dev.okhsunrog.vpnhide.ContextObservationInputs
import dev.okhsunrog.vpnhide.EffectTicket
import dev.okhsunrog.vpnhide.ObservationRuntime
import dev.okhsunrog.vpnhide.RootSnapshotCache
import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The production effects of a diagnostic run. Context observation goes through
 * the shared [RoutingGateCache] (which folds the refreshed root snapshot), so the
 * suite, the export gate and every screen read one routing source; probes are the
 * existing phased runners. Blocking helpers run on IO and are awaited, never
 * interrupted — the coordinator drains or quarantines them.
 */
internal class AppDiagnosticRunIo(
    private val inputs: () -> ContextObservationInputs?,
    private val clock: () -> Long = System::currentTimeMillis,
) : DiagnosticRunIo {
    override suspend fun observe(
        ticket: EffectTicket,
        stage: DiagnosticStage,
    ): DiagnosticContextObservation {
        val current = inputs() ?: return DiagnosticContextObservation(DiagnosticEligibility.Initializing, null)
        if (stage == DiagnosticStage.Checking) StartupTrace.mark("diagnostics_cache_start")
        // A pending self-restart is process readiness: the hooks are not in this
        // process, so no network fact could make a measurement meaningful.
        if (current.selfNeedsRestart) return DiagnosticContextObservation(DiagnosticEligibility.RestartApp, null)
        RoutingGateCache.ensureLoaded(ObservationRuntime.scope, current.context, selfNeedsRestart = false)
        // A not-invalidated observation is fresh; only a stale or absent one costs a root shell.
        when (routingReadPlan(RoutingGateCache.observation.value)) {
            RoutingRead.Reuse -> Unit
            RoutingRead.Join -> withContext(Dispatchers.IO) { RoutingGateCache.refreshInPlace(force = false) }
            RoutingRead.Refresh -> withContext(Dispatchers.IO) { RoutingGateCache.refreshInPlace(force = true) }
        }
        val observation =
            buildDiagnosticContextObservation(
                selfNeedsRestart = false,
                routing = RoutingGateCache.observation.value,
                snapshot = RootSnapshotCache.snapshot.value,
                config = CanonicalConfigRepository.state.value.confirmed,
                selfPackage = current.context.packageName,
                processIdentity = "pid:${Process.myPid()};uid:${Process.myUid()}",
                now = clock(),
            )
        if (stage == DiagnosticStage.Checking) traceEligibility(observation.eligibility)
        return observation
    }

    override suspend fun probe(
        ticket: EffectTicket,
        stage: DiagnosticStage,
        plan: List<ProbePlanEntry>,
    ): CheckResults =
        withContext(Dispatchers.IO) {
            val context = requireNotNull(inputs()) { "diagnostic inputs missing" }.context
            val cm = context.getSystemService(ConnectivityManager::class.java)
            when (stage) {
                DiagnosticStage.Core -> {
                    runCoreChecks(cm, context).also { StartupTrace.mark("diagnostics_cache_core_done") }
                }

                DiagnosticStage.Slow -> {
                    CheckResults(native = emptyList(), extraJava = runExtraJavaChecks(cm, context))
                        .also { StartupTrace.mark("diagnostics_cache_done") }
                }

                else -> {
                    error("stage $stage has no probes")
                }
            }
        }

    private fun traceEligibility(eligibility: DiagnosticEligibility) {
        when (eligibility) {
            DiagnosticEligibility.VpnOff -> StartupTrace.mark("diagnostics_cache_vpn_off")
            DiagnosticEligibility.SelfExcluded -> StartupTrace.mark("diagnostics_cache_self_not_routed")
            else -> Unit
        }
    }
}
