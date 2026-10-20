package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreProbeResult
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Which gate a failed startup root snapshot opens ([startupProbeFailure]). */
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
}
