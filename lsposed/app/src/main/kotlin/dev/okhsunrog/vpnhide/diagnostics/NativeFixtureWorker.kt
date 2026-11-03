package dev.okhsunrog.vpnhide.diagnostics

import uniffi.vpnhide_core_ffi.CoreNativeMeasurement
import uniffi.vpnhide_core_ffi.CoreNativeWorkerInstruction
import uniffi.vpnhide_core_ffi.CoreNativeWorkerPhase

/** Bounded observation adapter. Core owns admission, scope identity and verdict.
 * Keep this coroutine on one collector OS thread across every suspension.
 */
internal suspend fun measureNativeFixture(
    step: suspend (CoreNativeWorkerPhase) -> CoreNativeWorkerInstruction,
    register: (ULong) -> Unit,
    observe: () -> CoreNativeMeasurement,
): CoreNativeMeasurement {
    when (val next = step(CoreNativeWorkerPhase.Prepared)) {
        CoreNativeWorkerInstruction.ObserveProtected -> return observe()
        is CoreNativeWorkerInstruction.Register -> register(next.nonce)
        CoreNativeWorkerInstruction.ObserveRaw -> error("RAW observation requested before registration")
    }
    check(step(CoreNativeWorkerPhase.Registered) == CoreNativeWorkerInstruction.ObserveRaw)
    val raw = observe()
    // A lost/failed reply exits this adapter. It never guesses that RAW was
    // revoked, rolls a sample into the protected role, retries or rearms.
    check(step(CoreNativeWorkerPhase.RawMeasured(raw)) == CoreNativeWorkerInstruction.ObserveProtected)
    return observe()
}
