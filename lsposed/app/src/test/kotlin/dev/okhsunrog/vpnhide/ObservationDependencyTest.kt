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
import org.junit.Assert.assertNull
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList

class ObservationDependencyTest {
    @Test
    fun `slow derivation cannot publish an older root snapshot after shared refresh`() =
        runBlocking {
            val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
            try {
                withTimeout(5_000) {
                    val dependents = CopyOnWriteArrayList<ObservationCoordinator<RootProjection<String>>>()
                    val reads = Channel<Pair<ObservationRequest, CompletableDeferred<String>>>(Channel.UNLIMITED)
                    val root =
                        ObservationCoordinator(
                            scope = scope,
                            load = { request ->
                                val data = CompletableDeferred<String>()
                                reads.send(request to data)
                                RootSnapshot(mapOf("value" to data.await()), request.id, request.generation)
                            },
                            changed = { previous, next ->
                                if (rootObservationInvalidatesDependents(previous, next)) dependents.forEach { it.invalidate() }
                            },
                        )
                    val initial = async(start = CoroutineStart.UNDISPATCHED) { root.read() }
                    reads.receive().second.complete("old")
                    initial.await()
                    val slowStarted = CompletableDeferred<Unit>()
                    val slowRelease = CompletableDeferred<Unit>()
                    val slow =
                        ObservationCoordinator(scope, load = {
                            val snapshot = root.read()
                            if (snapshot.observationId == 1L) {
                                slowStarted.complete(Unit)
                                slowRelease.await()
                            }
                            RootProjection(snapshot.observationId, snapshot.generation, snapshot.sections.getValue("value"))
                        })
                    val fast =
                        ObservationCoordinator(scope, load = {
                            val snapshot = root.read()
                            RootProjection(snapshot.observationId, snapshot.generation, snapshot.sections.getValue("value"))
                        })
                    dependents.addAll(listOf(slow, fast))
                    val slowCaller = async(start = CoroutineStart.UNDISPATCHED) { slow.read() }
                    slowStarted.await()
                    assertEquals("old", fast.read().value)
                    val refreshed = async(start = CoroutineStart.UNDISPATCHED) { root.read(refresh = true) }
                    reads.receive().second.complete("new")
                    val source = refreshed.await()
                    fast.state.first { it.lastGood?.value?.observationId == source.observationId }
                    assertNull(slow.state.value.lastGood)
                    slowRelease.complete(Unit)
                    val slowValue = slowCaller.await()
                    assertEquals(source.observationId, slowValue.observationId)
                    assertEquals(slowValue, fast.read())
                    assertEquals("new", slowValue.value)
                    assertNull(reads.tryReceive().getOrNull())
                }
            } finally {
                scope.cancel()
            }
        }
}
