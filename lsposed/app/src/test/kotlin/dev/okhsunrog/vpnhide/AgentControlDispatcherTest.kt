package dev.okhsunrog.vpnhide

import kotlinx.serialization.decodeFromString
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentControlDispatcherTest {
    @Test
    fun exposesBridgeFunctions() {
        val names = AgentControlDispatcher.functionSpecs.map { it.name }

        assertTrue("getStatisticsState missing", "getStatisticsState" in names)
        assertTrue("getStatisticsCaptureBaseline missing", "getStatisticsCaptureBaseline" in names)
        assertTrue("getStatisticsCaptureDiff missing", "getStatisticsCaptureDiff" in names)
        assertTrue("setAppProtection missing", "setAppProtection" in names)
    }

    @Test
    fun functionsJsonRoundTrips() {
        val response = AgentBridgeJson.decodeFromString<AgentBridgeFunctionsResponse>(AgentControlDispatcher.functionsJson())
        val captureDiff = response.functions.single { it.name == "getStatisticsCaptureDiff" }

        assertTrue(captureDiff.description.contains("baseline"))
        assertEquals("object", captureDiff.inputSchema["type"].toString().trim('"'))
    }

    @Test
    fun callEnvelopeRoundTrips() {
        val call = AgentBridgeJson.decodeFromString<AgentBridgeCall>("""{"fn":"getDashboardState","args":{"refresh":true}}""")

        assertEquals("getDashboardState", call.fn)
        assertEquals("true", call.args["refresh"].toString())
    }
}
