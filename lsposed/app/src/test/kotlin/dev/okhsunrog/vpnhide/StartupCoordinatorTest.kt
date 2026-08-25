package dev.okhsunrog.vpnhide

import android.content.ContextWrapper
import dev.okhsunrog.vpnhide.startup.StartupCoordinator
import dev.okhsunrog.vpnhide.startup.StartupSelfTargetState
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class StartupCoordinatorTest {
    @Test
    fun `successful self target preparation seeds package list and cleanup boot id`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var seededInventory: PackageInventorySeed? = null
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
                            pmUsers = "UserInfo{0:Owner:c13}",
                        )
                    },
                    cleanupZygiskStatus = { _, bootId -> cleanupBootId = bootId },
                    seedRootSnapshotInventory = { seededInventory = it },
                    markStartupEvent = markers::add,
                )

            coordinator.prepareSelfTargets()

            assertEquals(StartupSelfTargetState.Ready(selfNeedsRestart = true), coordinator.selfTargetState.value)
            assertEquals(
                PackageInventorySeed(
                    packages = "package:dev.okhsunrog.vpnhide uid:10123",
                    users = "UserInfo{0:Owner:c13}",
                ),
                seededInventory,
            )
            assertEquals("boot-1", cleanupBootId)
            assertEquals(listOf("self_targets_start", "self_targets_done"), markers)
        }

    @Test
    fun `failed self target preparation reports error without seeding or cleanup`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var seededInventory: PackageInventorySeed? = null
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
                    seedRootSnapshotInventory = { seededInventory = it },
                    markStartupEvent = markers::add,
                )

            coordinator.prepareSelfTargets()

            assertEquals(
                StartupSelfTargetState.Failed(SelfTargetFailureKind.RootUnavailable, "exit=-1"),
                coordinator.selfTargetState.value,
            )
            assertNull(seededInventory)
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
