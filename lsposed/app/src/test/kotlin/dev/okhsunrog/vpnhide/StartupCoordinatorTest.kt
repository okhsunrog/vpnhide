package dev.okhsunrog.vpnhide

import android.content.ContextWrapper
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class StartupCoordinatorTest {
    @Test
    fun `successful self target preparation seeds package list and cleanup boot id`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var seededPackages: String? = null
            var cleanupBootId: String? = null
            val coordinator =
                StartupCoordinator(
                    appContext = FakeContext("dev.okhsunrog.vpnhide"),
                    prepareSelfTargetsCommand = { pkg ->
                        assertEquals("dev.okhsunrog.vpnhide", pkg)
                        SelfTargetPreparation(
                            rootAvailable = true,
                            selfNeedsRestart = true,
                            currentBootId = "boot-1",
                            pmPackages = "package:dev.okhsunrog.vpnhide uid:10123",
                        )
                    },
                    cleanupZygiskStatus = { _, bootId -> cleanupBootId = bootId },
                    seedRootSnapshotPackages = { seededPackages = it },
                    markStartupEvent = markers::add,
                )

            coordinator.prepareSelfTargets()

            assertEquals(StartupSelfTargetState.Ready(selfNeedsRestart = true), coordinator.selfTargetState.value)
            assertEquals("package:dev.okhsunrog.vpnhide uid:10123", seededPackages)
            assertEquals("boot-1", cleanupBootId)
            assertEquals(listOf("self_targets_start", "self_targets_done"), markers)
        }

    @Test
    fun `failed self target preparation reports error without seeding or cleanup`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var seededPackages: String? = null
            var cleanupBootId: String? = null
            val coordinator =
                StartupCoordinator(
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
                    seedRootSnapshotPackages = { seededPackages = it },
                    markStartupEvent = markers::add,
                )

            coordinator.prepareSelfTargets()

            assertEquals(
                StartupSelfTargetState.Failed(SelfTargetFailureKind.RootUnavailable, "exit=-1"),
                coordinator.selfTargetState.value,
            )
            assertNull(seededPackages)
            assertNull(cleanupBootId)
            assertEquals(
                listOf("self_targets_start", "self_targets_done", "self_targets_failed"),
                markers,
            )
        }

    private class FakeContext(
        private val packageName: String,
    ) : ContextWrapper(null) {
        override fun getPackageName(): String = packageName
    }
}
