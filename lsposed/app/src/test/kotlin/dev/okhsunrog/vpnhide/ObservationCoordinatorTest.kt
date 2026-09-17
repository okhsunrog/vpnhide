package dev.okhsunrog.vpnhide

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ObservationCoordinatorTest {
    @Test
    fun `fresh since handle cannot join an earlier collection`() =
        observationTest {
            val first = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            val old = reads.receive()
            val fresh =
                async(start = CoroutineStart.UNDISPATCHED) {
                    owner.read(refresh = true, notBefore = old.request.startedAt + 1)
                }
            old.result.complete("too early")
            val successor = reads.receive()
            assertTrue(successor.request.generation > old.request.generation)
            successor.result.complete("after request")
            assertEquals("after request", fresh.await())
            assertEquals("after request", first.await())
        }

    @Test
    fun `deferred invalidation hides old readiness and explicit read starts its generation`() =
        observationTest {
            val first = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            reads.receive().result.complete("old")
            first.await()
            owner.invalidate(start = false)
            assertNull(currentObservationValue(owner.state.value))
            assertNull(reads.tryReceive().getOrNull())
            val fresh = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            reads.receive().result.complete("new")
            assertEquals("new", fresh.await())
        }

    @Test
    fun `invalidation rejects old success and keeps waiters on the successor`() =
        observationTest {
            val first = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            val obsolete = reads.receive()
            repeat(3) { owner.invalidate() }
            val joined = async(start = CoroutineStart.UNDISPATCHED) { owner.read(refresh = true) }
            obsolete.result.complete("obsolete")
            val successor = reads.receive()
            assertEquals(3L, successor.request.generation)
            assertNull(owner.state.value.lastGood)
            assertTrue(owner.state.value.active != null)
            assertFalse(first.isCompleted)
            successor.result.complete("fresh")
            assertEquals("fresh", first.await())
            assertEquals("fresh", joined.await())
            assertNull(reads.tryReceive().getOrNull())
        }

    @Test
    fun `old failure cannot set an error or finish a new loading state`() =
        observationTest {
            val initial = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            reads.receive().result.complete("original")
            initial.await()
            val refresh = async(start = CoroutineStart.UNDISPATCHED) { owner.read(refresh = true) }
            val obsolete = reads.receive()
            owner.invalidate()
            obsolete.result.completeExceptionally(IllegalStateException("old failure"))
            val successor = reads.receive()
            val state = owner.state.value
            assertNull(state.error)
            assertEquals("original", state.lastGood?.value)
            assertNull(currentObservationValue(state))
            successor.result.complete("replacement")
            assertEquals("replacement", refresh.await())
        }

    @Test
    fun `concurrent equivalent refreshes join one physical read`() =
        observationTest {
            val first = async(start = CoroutineStart.UNDISPATCHED) { owner.read(refresh = true) }
            val read = reads.receive()
            val second = async(start = CoroutineStart.UNDISPATCHED) { owner.read(refresh = true) }
            owner.ensure()
            read.result.complete("shared")
            assertEquals("shared", first.await())
            assertEquals("shared", second.await())
            assertNull(reads.tryReceive().getOrNull())
        }

    @Test
    fun `cancelled waiter cannot cancel process owned read or poison its cache`() =
        observationTest {
            val caller = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            val read = reads.receive()
            caller.cancel()
            caller.join()
            read.result.complete("survives")
            owner.state.first { it.lastGood != null }
            assertEquals("survives", owner.read())
            assertNull(reads.tryReceive().getOrNull())
        }

    @Test
    fun `failure retains last good and ensure never retries it implicitly`() =
        observationTest {
            val first = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            reads.receive().result.complete("original")
            first.await()
            val failure = async(start = CoroutineStart.UNDISPATCHED) { runCatching { owner.read(refresh = true) } }
            reads.receive().result.completeExceptionally(IllegalStateException("failed"))
            assertTrue(failure.await().isFailure)
            repeat(3) { owner.ensure() }
            assertNull(reads.tryReceive().getOrNull())
            assertEquals(
                "original",
                owner.state.value.lastGood
                    ?.value,
            )
            assertNull(currentObservationValue(owner.state.value))
            val retry = async(start = CoroutineStart.UNDISPATCHED) { owner.read(refresh = true) }
            reads.receive().result.complete("recovered")
            assertEquals("recovered", retry.await())
        }

    @Test
    fun `missing inputs remain pristine and can be initialized later`() =
        observationTest(initiallyReady = false) {
            owner.ensure()
            owner.invalidate()
            val failure = runCatching { owner.read(refresh = true) }.exceptionOrNull() as ObservationReadException
            assertEquals(TransitionFailure.InitializationPending, failure.reason)
            assertFalse(owner.state.value.attempted)
            assertNull(owner.state.value.error)
            ready = true
            val caller = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            reads.receive().result.complete("initialized")
            assertEquals("initialized", caller.await())
        }

    @Test
    fun `deadline quarantines worker and a retry waits for actual drain`() =
        observationTest {
            val caller = async(start = CoroutineStart.UNDISPATCHED) { runCatching { owner.read() } }
            val hung = reads.receive()
            deadlines.receive().complete(Unit)
            val failure = caller.await().exceptionOrNull() as ObservationReadException
            assertEquals(TransitionFailure.DeadlineExceeded, failure.reason)
            assertTrue(owner.state.value.quarantined)
            val rejected = runCatching { owner.read(refresh = true) }.exceptionOrNull() as ObservationReadException
            assertEquals(TransitionFailure.ResourceUnavailable, rejected.reason)
            assertNull(reads.tryReceive().getOrNull())
            hung.result.complete("too late")
            val retry = reads.receive()
            assertNull(owner.state.value.lastGood)
            retry.result.complete("after drain")
            owner.state.first { it.lastGood != null }
            assertEquals("after drain", owner.read())
        }

    @Test
    fun `draining timed out worker does not create an automatic retry loop`() =
        observationTest {
            val caller = async(start = CoroutineStart.UNDISPATCHED) { runCatching { owner.read() } }
            val hung = reads.receive()
            deadlines.receive().complete(Unit)
            caller.await()
            hung.result.complete("too late")
            owner.state.first { !it.quarantined }
            assertNull(owner.state.value.lastGood)
            assertNull(owner.state.value.active)
            assertNull(reads.tryReceive().getOrNull())
            assertEquals(TransitionFailure.DeadlineExceeded, owner.state.value.error)
        }

    @Test
    fun `read reasons reach the request and a background cause never outranks the user`() =
        observationTest {
            val first = async(start = CoroutineStart.UNDISPATCHED) { owner.read() }
            reads.receive().result.complete("old")
            first.await()
            owner.invalidate(start = false, reason = ReadReason.Transition)
            owner.invalidate(start = false)
            assertEquals(
                ReadReason.Transition,
                owner.state.value.stale
                    ?.reason,
            )
            assertNull(reads.tryReceive().getOrNull())
            val explicit = async(start = CoroutineStart.UNDISPATCHED) { owner.read(refresh = true) }
            val read = reads.receive()
            assertEquals(ReadReason.Explicit, read.request.reason)
            assertEquals(
                ReadReason.Explicit,
                owner.state.value.stale
                    ?.reason,
            )
            read.result.complete("new")
            assertEquals("new", explicit.await())
            assertNull(owner.state.value.stale)
        }

    private fun observationTest(
        initiallyReady: Boolean = true,
        block: suspend Fixture.() -> Unit,
    ) = runBlocking {
        val fixture = Fixture(initiallyReady)
        try {
            withTimeout(5_000) { fixture.block() }
        } finally {
            fixture.scope.cancel()
        }
    }

    private class Read(
        val request: ObservationRequest,
        val result: CompletableDeferred<String> = CompletableDeferred(),
    )

    private class Fixture(
        @Volatile var ready: Boolean,
    ) : CoroutineScope by CoroutineScope(SupervisorJob() + Dispatchers.Default) {
        val scope: CoroutineScope get() = this
        val reads = Channel<Read>(Channel.UNLIMITED)
        val deadlines = Channel<CompletableDeferred<Unit>>(Channel.UNLIMITED)
        val owner =
            ObservationCoordinator(
                scope = scope,
                ready = { ready },
                load = { request -> Read(request).also { reads.send(it) }.result.await() },
                deadline = { CompletableDeferred<Unit>().also { deadlines.send(it) }.await() },
            )
    }
}
