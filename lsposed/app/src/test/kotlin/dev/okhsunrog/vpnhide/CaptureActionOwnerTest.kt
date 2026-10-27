package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.debug.CaptureActionOwner
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class CaptureActionOwnerTest {
    @Test
    fun `cancelled stop waiter cannot drop accepted work or let save overtake it`() =
        runTest {
            val owner = CaptureActionOwner(backgroundScope)
            val coreStopped = CompletableDeferred<Unit>()
            val sequence = mutableListOf<String>()
            val stop =
                async {
                    owner.perform {
                        sequence += "stop requested"
                        coreStopped.await()
                        sequence += "core stopped"
                    }
                }
            runCurrent()
            stop.cancel()
            val save = async { owner.perform { sequence += "saved" } }
            runCurrent()
            assertEquals(listOf("stop requested"), sequence)
            assertFalse(save.isCompleted)
            coreStopped.complete(Unit)
            runCurrent()
            save.await()
            assertEquals(listOf("stop requested", "core stopped", "saved"), sequence)
            assertTrue(stop.isCancelled)
        }
}
