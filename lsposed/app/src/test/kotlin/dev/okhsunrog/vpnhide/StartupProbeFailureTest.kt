package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreProbeResult
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Which gate a failed startup root snapshot opens ([startupProbeFailure]). */
@OptIn(ExperimentalCoroutinesApi::class)
internal class StartupProbeFailureTest {
    @Test
    fun `core not reachable is the missing-core gate`() {
        assertEquals(
            SelfTargetFailureKind.RootUnavailable to "unreachable: connection refused",
            startupProbeFailure(CoreDiagnostics.Failed("unreachable", "connection refused")),
        )
    }

    @Test
    fun `a protocol skew is the version-mismatch gate, not a missing core`() {
        val detail = "protocol skew: core=5, app=6 — update the lagging component"
        assertEquals(
            SelfTargetFailureKind.CoreVersionMismatch to "protocol_skew: $detail",
            startupProbeFailure(CoreDiagnostics.Failed("protocol_skew", detail)),
        )
    }

    @Test
    fun `core answered but the probe failed is retryable, not a missing core`() {
        assertEquals(
            SelfTargetFailureKind.Unknown to "core root snapshot probe exit=1",
            startupProbeFailure(CoreDiagnostics.Collected(listOf(CoreProbeResult(1, "")), dmesgCleared = false)),
        )
    }

    @Test
    fun `a successful snapshot is no failure`() {
        assertNull(startupProbeFailure(CoreDiagnostics.Collected(listOf(CoreProbeResult(0, "x")), dmesgCleared = false)))
    }

    @Test
    fun `busy detail and truncated successful output remain failures`() {
        assertEquals(
            SelfTargetFailureKind.Unknown to "core root snapshot probe exit=-1: core diagnostics busy",
            startupProbeFailure(CoreDiagnostics.Collected(listOf(CoreProbeResult(-1, "core diagnostics busy")), false)),
        )
        assertEquals(
            SelfTargetFailureKind.IncompleteData to "core root snapshot probe output truncated",
            startupProbeFailure(CoreDiagnostics.Collected(listOf(CoreProbeResult(0, "partial", truncated = true)), false)),
        )
    }

    @Test
    fun `startup joins shared inventory read and invalidation waits for its physical completion`() =
        runTest {
            val release = CompletableDeferred<Unit>()
            var calls = 0
            val owner =
                ObservationCoordinator(backgroundScope, load = { request ->
                    calls++
                    if (calls == 1) release.await()
                    RootSnapshot(mapOf("value" to calls.toString()), request.id, request.generation)
                })
            val startup = async { loadStartupRootSections { owner.read() } }
            val inventory = async { owner.read() }
            runCurrent()
            assertEquals(1, calls)
            owner.invalidate()
            runCurrent()
            assertEquals(1, calls)
            release.complete(Unit)
            runCurrent()
            assertEquals(2, calls)
            assertEquals(mapOf("value" to "2"), startup.await().first)
            assertEquals(inventory.await().sections, startup.await().first)
            assertNull(startup.await().second)
        }

    @Test
    fun `shared read owner preserves typed failure and Retry replaces it without stale error`() =
        runTest {
            var calls = 0
            val owner =
                ObservationCoordinator(backgroundScope, load = { _: ObservationRequest ->
                    calls++
                    if (calls == 1) throw RootSnapshotException("protocol skew", SelfTargetFailureKind.CoreVersionMismatch)
                    RootSnapshot(mapOf("ready" to "true"))
                })
            val failed = async { loadStartupRootSections { owner.read() } }
            runCurrent()
            assertEquals(SelfTargetFailureKind.CoreVersionMismatch to "protocol skew", failed.await().second)
            owner.invalidate(start = false)
            val retry = async { loadStartupRootSections { owner.read() } }
            runCurrent()
            assertEquals(mapOf("ready" to "true"), retry.await().first)
            assertNull(retry.await().second)
            assertNull(owner.state.value.errorCause)
        }

    @Test
    fun `Retry after package change performs a full Core inventory read instead of grafting old packages`() =
        runTest {
            var packages = "package:target uid:10123"
            var calls = 0
            dev.okhsunrog.vpnhide.core.CoreRoot.setTransport(
                object : TestCoreRootTransport() {
                    override suspend fun collectDiagnostics(
                        request: uniffi.vpnhide_core_ffi.CoreDiagnosticsRequest,
                    ): uniffi.vpnhide_core_ffi.CoreDiagnosticsReport {
                        val probe = request.probes.single() as uniffi.vpnhide_core_ffi.CoreDiagnosticProbe.RootSnapshot
                        org.junit.Assert.assertTrue(probe.packages)
                        calls++
                        val raw =
                            REQUIRED_ROOT_SNAPSHOT_SECTIONS.joinToString("\n") { section ->
                                "__VPNHIDE_ROOT_SECTION_BEGIN__:$section\n" +
                                    (if (section == "pm_packages") packages else "") +
                                    "\n__VPNHIDE_ROOT_SECTION_END__:$section"
                            }
                        return uniffi.vpnhide_core_ffi.CoreDiagnosticsReport(
                            listOf(uniffi.vpnhide_core_ffi.CoreProbeOutput(probe, 0, raw, null, emptyList(), false)),
                            false,
                        )
                    }
                },
            )
            try {
                val owner =
                    ObservationCoordinator(backgroundScope, load = { _: ObservationRequest ->
                        RootSnapshot(loadRootShellSnapshot(recordMetric = { _, _ -> }))
                    })
                val initial = async { loadStartupRootSections { owner.read() } }
                runCurrent()
                assertEquals(packages, initial.await().first?.get("pm_packages"))
                packages = "package:target uid:10123,110123"
                owner.invalidate(start = false)
                val retry = async { loadStartupRootSections { owner.read() } }
                runCurrent()
                assertEquals(packages, retry.await().first?.get("pm_packages"))
                assertEquals(2, calls)
            } finally {
                dev.okhsunrog.vpnhide.core.CoreRoot
                    .setTransport(null)
            }
        }
}
