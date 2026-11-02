package dev.okhsunrog.vpnhide

import android.content.ContextWrapper
import dev.okhsunrog.vpnhide.startup.StartupCoordinator
import dev.okhsunrog.vpnhide.startup.StartupSelfTargetState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class StartupCoordinatorTest {
    @Test
    fun `recreated Activity joins preparation after previous waiter is cancelled`() =
        runBlocking {
            withTimeout(10_000) {
                val started = CompletableDeferred<Unit>()
                val release = CompletableDeferred<Unit>()
                var calls = 0
                val coordinator =
                    StartupCoordinator(
                        appContext = FakeContext("dev.okhsunrog.vpnhide"),
                        initializeConfig = {},
                        prepareSelfTargetsCommand = {
                            calls += 1
                            started.complete(Unit)
                            release.await()
                            SelfTargetPreparation(rootAvailable = true, selfNeedsRestart = false, currentBootId = "boot-1")
                        },
                        cleanupZygiskStatus = { _, _ -> },
                        markStartupEvent = {},
                    )
                val previous = async { coordinator.prepareSelfTargets() }
                started.await()
                previous.cancel()
                val recreated = async { coordinator.prepareSelfTargets() }
                release.complete(Unit)
                recreated.await()
                coordinator.prepareSelfTargets()
                assertEquals(1, calls)
                assertEquals(StartupSelfTargetState.Ready(false), coordinator.selfTargetState.value)
            }
        }

    @Test
    fun `successful shared preparation records cleanup boot identity`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var cleanupBootId: String? = null
            val coordinator =
                StartupCoordinator(
                    initializeConfig = {},
                    appContext = FakeContext("dev.okhsunrog.vpnhide"),
                    prepareSelfTargetsCommand = { pkg ->
                        assertEquals("dev.okhsunrog.vpnhide", pkg)
                        SelfTargetPreparation(
                            rootAvailable = true,
                            selfNeedsRestart = true,
                            currentBootId = "boot-1",
                        )
                    },
                    cleanupZygiskStatus = { _, bootId -> cleanupBootId = bootId },
                    markStartupEvent = markers::add,
                )

            coordinator.prepareSelfTargets()

            assertEquals(StartupSelfTargetState.Ready(selfNeedsRestart = true), coordinator.selfTargetState.value)
            assertEquals("boot-1", cleanupBootId)
            assertEquals(listOf("self_targets_start", "config_init_done", "self_targets_done"), markers)
        }

    @Test
    fun `failed self target preparation reports error without seeding or cleanup`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var cleanupBootId: String? = null
            val coordinator =
                StartupCoordinator(
                    initializeConfig = {},
                    appContext = FakeContext("dev.okhsunrog.vpnhide"),
                    prepareSelfTargetsCommand = {
                        SelfTargetPreparation(
                            rootAvailable = false,
                            selfNeedsRestart = false,
                            currentBootId = null,
                            error = "exit=-1",
                            failureKind = SelfTargetFailureKind.RootUnavailable,
                        )
                    },
                    cleanupZygiskStatus = { _, bootId -> cleanupBootId = bootId },
                    markStartupEvent = markers::add,
                )

            coordinator.prepareSelfTargets()

            assertEquals(
                StartupSelfTargetState.Failed(SelfTargetFailureKind.RootUnavailable, "exit=-1"),
                coordinator.selfTargetState.value,
            )
            assertNull(cleanupBootId)
            assertEquals(
                listOf("self_targets_start", "config_init_done", "self_targets_done", "self_targets_failed"),
                markers,
            )
        }

    @Test
    fun `late Open and reconnect reuse successful preparation and cannot clear restart requirement`() =
        runTest {
            val availability = MutableStateFlow(ConfigCoordinatorMode.Initializing)
            var calls = 0
            var invalidations = 0
            val coordinator =
                StartupCoordinator(
                    appContext = FakeContext("dev.okhsunrog.vpnhide"),
                    owner = backgroundScope,
                    preparationDispatcher = kotlinx.coroutines.test.StandardTestDispatcher(testScheduler),
                    availability = availability,
                    initializeConfig = {},
                    prepareSelfTargetsCommand = {
                        calls++
                        SelfTargetPreparation(true, calls == 1, "boot-1")
                    },
                    cleanupZygiskStatus = { _, _ -> },
                    invalidateRootSnapshot = { invalidations++ },
                    markStartupEvent = {},
                )
            coordinator.observeAvailability()
            coordinator.prepareSelfTargets()
            availability.value = ConfigCoordinatorMode.Open
            runCurrent()
            availability.value = ConfigCoordinatorMode.Unavailable
            runCurrent()
            availability.value = ConfigCoordinatorMode.Open
            runCurrent()
            assertEquals(1, calls)
            coordinator.prepareSelfTargets(force = true)
            assertEquals(2, calls)
            assertEquals(1, invalidations)
            assertEquals(StartupSelfTargetState.Ready(true), coordinator.selfTargetState.value)
        }

    @Test
    fun `reconnect after failed startup requests one fresh preparation and concurrent Retry joins it`() =
        runTest {
            val availability = MutableStateFlow(ConfigCoordinatorMode.Unavailable)
            val release = CompletableDeferred<Unit>()
            var calls = 0
            var invalidations = 0
            val coordinator =
                StartupCoordinator(
                    appContext = FakeContext("dev.okhsunrog.vpnhide"),
                    owner = backgroundScope,
                    preparationDispatcher = kotlinx.coroutines.test.StandardTestDispatcher(testScheduler),
                    availability = availability,
                    initializeConfig = {},
                    prepareSelfTargetsCommand = {
                        calls++
                        if (calls == 1) {
                            SelfTargetPreparation(false, false, null, error = "unreachable")
                        } else {
                            release.await()
                            SelfTargetPreparation(true, false, "boot-1")
                        }
                    },
                    cleanupZygiskStatus = { _, _ -> },
                    invalidateRootSnapshot = { invalidations++ },
                    markStartupEvent = {},
                )
            coordinator.observeAvailability()
            coordinator.prepareSelfTargets()
            availability.value = ConfigCoordinatorMode.Open
            runCurrent()
            val retry = async { coordinator.prepareSelfTargets(force = true) }
            runCurrent()
            assertEquals(2, calls)
            assertEquals(1, invalidations)
            release.complete(Unit)
            retry.await()
            assertEquals(StartupSelfTargetState.Ready(false), coordinator.selfTargetState.value)
        }

    private class FakeContext(
        private val packageName: String,
    ) : ContextWrapper(null) {
        override fun getPackageName(): String = packageName
    }
}
