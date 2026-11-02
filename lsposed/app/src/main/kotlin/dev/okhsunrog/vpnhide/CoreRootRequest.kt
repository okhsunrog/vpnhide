package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreDiagnosticException
import uniffi.vpnhide_core_ffi.CoreException

/** Diagnostic refusals leave the connection intact; transport failures share
 * the command/probe publication owner and preserve its generation fence. */
internal suspend fun <T> coreRootRequest(
    request: Long,
    failed: (Long, CoreException) -> Unit,
    block: suspend () -> T,
): T =
    try {
        block()
    } catch (error: CoreException) {
        failed(request, error)
        throw error
    } catch (error: CoreDiagnosticException.Transport) {
        failed(request, error.error)
        throw error.error
    }
