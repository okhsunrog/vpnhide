package dev.okhsunrog.vpnhide.debug

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.async
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Accepted capture actions are serialized on the process scope; a waiter only detaches. */
internal class CaptureActionOwner(
    private val scope: CoroutineScope,
) {
    private val mutex = Mutex()

    suspend fun <T> perform(action: suspend () -> T): T = scope.async { mutex.withLock { action() } }.await()
}
