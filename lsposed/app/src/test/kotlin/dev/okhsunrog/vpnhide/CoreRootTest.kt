package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import dev.okhsunrog.vpnhide.core.CoreRootTransport
import dev.okhsunrog.vpnhide.core.coreDiagnostics
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import uniffi.vpnhide_core_ffi.CoreDiagnosticProbe
import uniffi.vpnhide_core_ffi.CoreDiagnosticsReport
import uniffi.vpnhide_core_ffi.CoreDiagnosticsRequest
import uniffi.vpnhide_core_ffi.CoreLogcatRequest
import uniffi.vpnhide_core_ffi.CoreProbeOutput
import uniffi.vpnhide_core_ffi.LogcatReply

internal open class TestCoreRootTransport : CoreRootTransport {
    override suspend fun collectDiagnostics(request: CoreDiagnosticsRequest): CoreDiagnosticsReport = error("unexpected diagnostics")

    override suspend fun logcatRecording(request: CoreLogcatRequest): LogcatReply = error("unexpected logcat request")

    override suspend fun acquireDebugLease(id: Long) = Unit

    override suspend fun releaseDebugLease(id: Long) = Unit
}

class CoreRootTest {
    @get:Rule val tmp = TemporaryFolder()

    @After fun detach() = CoreRoot.setTransport(null)

    @Test
    fun `probe commands use generated enums and preserve ordered forensic payloads`() =
        runTest {
            val probes = listOf(CoreDiagnosticProbe.Dmesg, CoreDiagnosticProbe.AppVpnState(10123u))
            CoreRoot.setTransport(
                object : TestCoreRootTransport() {
                    override suspend fun collectDiagnostics(request: CoreDiagnosticsRequest): CoreDiagnosticsReport {
                        assertEquals(probes, request.probes)
                        assertTrue(request.clearDmesg)
                        return CoreDiagnosticsReport(
                            request.probes.map { CoreProbeOutput(it, 0, "forensic text", null, emptyList(), false) },
                            true,
                        )
                    }
                },
            )
            val result = CoreRoot.collect(probes, clearDmesg = true) as CoreDiagnostics.Collected
            assertEquals(listOf("forensic text", "forensic text"), result.results.map { it.text })
            assertTrue(result.dmesgCleared)
        }

    @Test
    fun `unattached core never returns an empty successful probe`() =
        runTest {
            val result = CoreRoot.collect(listOf(CoreDiagnosticProbe.HookCounters))
            assertTrue(result is CoreDiagnostics.Failed)
            assertEquals(-1, result.results(1).single().exit)
        }

    @Test
    fun `exported content is read once and removed and short report is refused`() {
        val file = tmp.newFile("snapshot.txt").apply { writeText("big output") }
        val output = CoreProbeOutput(CoreDiagnosticProbe.DebugSnapshot, 0, "", file.absolutePath, listOf("/x/manifest.txt"), true)
        val report = CoreDiagnosticsReport(listOf(output), false)
        val result = (coreDiagnostics(report, 1) as CoreDiagnostics.Collected).results.single()
        assertEquals("big output", result.text)
        assertTrue(result.truncated)
        assertFalse(file.exists())
        assertTrue(coreDiagnostics(report, 2) is CoreDiagnostics.Failed)
    }

    @Test
    fun `superkey request cannot expose secret through its local intent description`() {
        val key = CoreRootEffect.SetSuperkey("s3cret")
        assertFalse(key.toString().contains("s3cret"))
        assertTrue(key.valid)
        listOf("", "  ", "a\nb", "a\u0000b", "x".repeat(4097)).forEach { assertFalse(CoreRootEffect.SetSuperkey(it).valid) }
    }
}
