package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.checks.AppVpnState
import dev.okhsunrog.vpnhide.checks.AppVpnStateResponse
import dev.okhsunrog.vpnhide.checks.CheckStatus
import dev.okhsunrog.vpnhide.checks.ChecksResponse
import dev.okhsunrog.vpnhide.checks.KpmListResponse
import dev.okhsunrog.vpnhide.checks.NativeProbe
import dev.okhsunrog.vpnhide.checks.ObservationError
import dev.okhsunrog.vpnhide.checks.RoutingResponse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeProbeContractTest {
    @Test
    fun `combined app VPN state preserves session and UID membership`() {
        val response =
            NativeProbe.parseAppVpnState(fixture("app-vpn-state-routed.json")) as AppVpnStateResponse.Success
        assertEquals(AppVpnState.ROUTED, response.observation.state)
        assertEquals("framework:143:tun0", response.observation.session)
        assertEquals(listOf("tun0"), response.observation.interfaces)
    }

    @Test
    fun `known checks fixture preserves the Rust payload fields`() {
        val response = NativeProbe.parseChecks(fixture("checks-pass.json")) as ChecksResponse.Success
        val check = response.checks.getValue("ioctl_flags")

        assertEquals(CheckStatus.PASS, check.status)
        assertEquals("no VPN", check.detail)
    }

    @Test
    fun `empty checks fixture is a successful empty observation`() {
        val response = NativeProbe.parseChecks(fixture("checks-empty.json"))

        assertEquals(ChecksResponse.Success(emptyMap()), response)
    }

    @Test
    fun `unknown check status remains unknown`() {
        val response = NativeProbe.parseChecks(fixture("checks-unknown-status.json")) as ChecksResponse.Success

        assertEquals(CheckStatus.UNKNOWN, response.checks.getValue("future_probe").status)
    }

    @Test
    fun `routing null is preserved as an inconclusive observation`() {
        val response = NativeProbe.parseRouting(fixture("routing-null.json")) as RoutingResponse.Success

        assertEquals(10042L, response.observation.uid)
        assertEquals(null, response.observation.routed)
    }

    @Test
    fun `routing string boolean is rejected`() {
        val response = NativeProbe.parseRouting(fixture("routing-string-boolean.json")) as RoutingResponse.Failure

        assertEquals(ObservationError.Malformed, response.error)
    }

    @Test
    fun `kpm empty and unavailable stay distinct`() {
        val empty = NativeProbe.parseKpmList(fixture("kpm-empty.json")) as KpmListResponse.Success
        val unavailable = NativeProbe.parseKpmList(fixture("kpm-unavailable.json")) as KpmListResponse.Failure
        val malformed = NativeProbe.parseKpmList(fixture("kpm-malformed.json")) as KpmListResponse.Failure

        assertTrue(empty.observation.available)
        assertTrue(empty.observation.modules.isEmpty())
        assertEquals(ObservationError.Unavailable, unavailable.error)
        assertEquals(ObservationError.Malformed, malformed.error)
    }

    @Test
    fun `malformed and unsupported responses are classified before data parsing`() {
        val malformed = NativeProbe.parseChecks(fixture("malformed-truncated.json")) as ChecksResponse.Failure
        val unsupported = NativeProbe.parseChecks(fixture("unsupported-version.json")) as ChecksResponse.Failure

        assertEquals(ObservationError.Malformed, malformed.error)
        assertEquals(ObservationError.UnsupportedVersion(99), unsupported.error)
    }

    @Test
    fun `unknown envelope status is not mapped to a known error`() {
        val response =
            NativeProbe.parseChecks(
                """
                {"version":1,"kind":"checks","status":"future_status","data":[]}
                """.trimIndent(),
            ) as ChecksResponse.Failure

        assertEquals(ObservationError.UnknownStatus("future_status"), response.error)
    }

    private fun fixture(name: String): String = javaClass.getResourceAsStream("/$name")!!.bufferedReader().use { it.readText() }
}
