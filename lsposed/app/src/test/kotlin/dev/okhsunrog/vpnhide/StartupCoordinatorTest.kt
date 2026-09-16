package dev.okhsunrog.vpnhide

import android.content.ContextWrapper
import dev.okhsunrog.vpnhide.startup.StartupCoordinator
import dev.okhsunrog.vpnhide.startup.StartupSelfTargetState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

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
                        seedRootSnapshotInventory = {},
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
    fun `successful self target preparation seeds package list and cleanup boot id`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var seededInventory: PackageInventorySeed? = null
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
            assertEquals(listOf("self_targets_start", "config_init_done", "self_targets_done"), markers)
        }

    @Test
    fun `a preparation that wrote nothing seeds the whole snapshot instead of the inventory`() =
        runBlocking {
            var seededInventory: PackageInventorySeed? = null
            var seededSections: Map<String, String>? = null
            val sections = mapOf("current_boot_id" to "boot-1", "pm_packages" to "package:a uid:1", "pm_users" to "UserInfo{0:O:c13}")
            val coordinator =
                StartupCoordinator(
                    initializeConfig = {},
                    appContext = FakeContext("dev.okhsunrog.vpnhide"),
                    prepareSelfTargetsCommand = {
                        SelfTargetPreparation(
                            rootAvailable = true,
                            selfNeedsRestart = false,
                            currentBootId = "boot-1",
                            pmPackages = "package:a uid:1",
                            pmUsers = "UserInfo{0:O:c13}",
                            sections = sections,
                        )
                    },
                    cleanupZygiskStatus = { _, _ -> },
                    seedRootSnapshotInventory = { seededInventory = it },
                    seedRootSnapshot = { seededSections = it },
                    markStartupEvent = {},
                )

            coordinator.prepareSelfTargets()

            assertEquals(sections, seededSections)
            assertNull(seededInventory)
        }

    @Test
    fun `failed self target preparation reports error without seeding or cleanup`() =
        runBlocking {
            val markers = mutableListOf<String>()
            var seededInventory: PackageInventorySeed? = null
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
                listOf("self_targets_start", "config_init_done", "self_targets_done", "self_targets_failed"),
                markers,
            )
        }

    private class FakeContext(
        private val packageName: String,
    ) : ContextWrapper(null) {
        override fun getPackageName(): String = packageName
    }
}
