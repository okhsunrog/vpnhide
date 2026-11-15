package dev.okhsunrog.vpnhide.diagnostics

import uniffi.vpnhide_core_ffi.CoreDiagnosticException
import uniffi.vpnhide_core_ffi.CoreDiagnosticWorkerTask
import uniffi.vpnhide_core_ffi.CorePortsWorkerPhase
import uniffi.vpnhide_core_ffi.CoreSession
import uniffi.vpnhide_core_ffi.observePortsFixture

internal suspend fun measurePortsFixture(
    core: CoreSession,
    task: CoreDiagnosticWorkerTask,
) {
    try {
        val before =
            workerStage("ports-prepare-${task.session}") {
                core.portsWorkerStep(task, CorePortsWorkerPhase.Prepared)
            } ?: return
        val protected =
            workerStage("ports-protect-${task.session}") {
                core.portsWorkerStep(task, CorePortsWorkerPhase.Protected(observePortsFixture(before)))
            } ?: return
        val revoked =
            workerStage("ports-revoke-${task.session}") {
                core.portsWorkerStep(task, CorePortsWorkerPhase.Revoked(observePortsFixture(protected)))
            } ?: return
        workerStage("ports-finish-${task.session}") {
            core.portsWorkerStep(task, CorePortsWorkerPhase.Finished(observePortsFixture(revoked)))
        }
    } catch (_: CoreDiagnosticException) {
        // Core retains the partial observations and owns removal of any installed rules.
    }
}
