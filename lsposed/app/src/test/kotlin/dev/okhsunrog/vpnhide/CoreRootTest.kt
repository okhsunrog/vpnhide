package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreDiagnostics
import dev.okhsunrog.vpnhide.core.CoreEffectOutcome
import dev.okhsunrog.vpnhide.core.CoreProbe
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.CoreRootEffect
import dev.okhsunrog.vpnhide.core.CoreRootTransport
import dev.okhsunrog.vpnhide.core.parseCoreDiagnostics
import dev.okhsunrog.vpnhide.core.parseCoreEffectReply
import kotlinx.coroutines.test.runTest
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** The app's only route to root: typed probes and effects through core. */
class CoreRootTest {
    @get:Rule val tmp = TemporaryFolder()

    @After fun detach() = CoreRoot.setTransport(null)

    @Test
    fun `requests name probes and effects by core's wire tags only`() =
        runTest {
            val sent = mutableListOf<String>()
            CoreRoot.setTransport(
                object : CoreRootTransport {
                    override suspend fun collectDiagnostics(requestJson: String): String {
                        sent += requestJson
                        return """{"status":"ok","report":{"outputs":[{"probe":{"kind":"dmesg"},"exit":0,"output":"ring"},""" +
                            """{"probe":{"kind":"app_vpn_state","uid":10123},"exit":0,"output":"log"}],"dmesg_cleared":true}}"""
                    }

                    override suspend fun rootEffect(
                        op: Long,
                        effectJson: String,
                    ): String {
                        sent += effectJson
                        return """{"status":"ok"}"""
                    }
                },
            )
            val report = CoreRoot.collect(listOf(CoreProbe.Dmesg, CoreProbe.AppVpnState(10123)), clearDmesg = true)
            val request = JSONObject(sent[0])
            assertTrue(request.getBoolean("clear_dmesg"))
            assertEquals("dmesg", request.getJSONArray("probes").getJSONObject(0).getString("kind"))
            assertEquals(10123, request.getJSONArray("probes").getJSONObject(1).getInt("uid"))
            assertEquals(listOf("ring", "log"), report.results(2).map { it.text })
            assertTrue((report as CoreDiagnostics.Collected).dmesgCleared)

            assertEquals(CoreEffectOutcome.Ok, CoreRoot.apply(5, CoreRootEffect.SetSuperkey("abc")))
            assertEquals(JSONObject("""{"kind":"set_superkey","superkey":"abc"}""").toString(), JSONObject(sent[1]).toString())
        }

    @Test
    fun `an unattached or failing core reads as unavailable, never as empty success`() =
        runTest {
            val failed = CoreRoot.collect(listOf(CoreProbe.HookCounters))
            assertTrue(failed is CoreDiagnostics.Failed)
            assertEquals(-1, failed.results(1).single().exit)
            assertEquals(CoreEffectOutcome.FailedKnown, CoreRoot.apply(1, CoreRootEffect.ClearSuperkey))
        }

    @Test
    fun `exported output is read back from core's file and removed`() {
        val file = tmp.newFile("debug_snapshot-1.txt").apply { writeText("big output") }
        val raw =
            """{"status":"ok","report":{"outputs":[{"probe":{"kind":"debug_snapshot"},"exit":0,"output":"",""" +
                """"file":"${file.absolutePath}","artifacts":["/x/manifest.txt"],"truncated":true}],"dmesg_cleared":false}}"""
        val result = (parseCoreDiagnostics(raw, 1) as CoreDiagnostics.Collected).results.single()
        assertEquals("big output", result.text)
        assertTrue(result.truncated)
        assertEquals("manifest.txt", result.artifacts.single().name)
        assertFalse(file.exists())
    }

    @Test
    fun `malformed or short reports fail closed`() {
        assertTrue(parseCoreDiagnostics("""{"status":"ok","report":{"outputs":[]}}""", 1) is CoreDiagnostics.Failed)
        assertTrue(parseCoreDiagnostics("""{"status":"unreachable"} trailing""", 0) is CoreDiagnostics.Failed)
        assertEquals("protocol_skew", (parseCoreDiagnostics("""{"status":"protocol_skew"}""", 0) as CoreDiagnostics.Failed).status)
    }

    @Test
    fun `effect replies map refusals to known failures and lost replies to unknown`() {
        assertEquals(CoreEffectOutcome.Ok, parseCoreEffectReply("""{"status":"ok"}"""))
        listOf("busy", "refused", "failed", "invalid_input", "unreachable", "protocol_skew").forEach {
            assertEquals(CoreEffectOutcome.FailedKnown, parseCoreEffectReply("""{"status":"$it"}"""))
        }
        assertEquals(CoreEffectOutcome.Unknown, parseCoreEffectReply("""{"status":"transport"}"""))
        assertEquals(CoreEffectOutcome.Unknown, parseCoreEffectReply("garbage"))
    }

    @Test
    fun `superkey effects keep the key out of toString and mirror core's input rule`() {
        val key = CoreRootEffect.SetSuperkey("s3cret")
        assertFalse(key.toString().contains("s3cret"))
        assertTrue(key.valid)
        listOf("", "  ", "a\nb", "a\u0000b", "x".repeat(4097)).forEach { assertFalse(CoreRootEffect.SetSuperkey(it).valid) }
    }
}
