package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreLogcatReply
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.CoreRootTransport
import dev.okhsunrog.vpnhide.core.LogcatAutoStop
import dev.okhsunrog.vpnhide.core.LogcatDebugRestore
import dev.okhsunrog.vpnhide.core.parseCoreLogcatReply
import dev.okhsunrog.vpnhide.core.parseCoreLogcatSession
import dev.okhsunrog.vpnhide.debug.DebugCaptureInfo
import dev.okhsunrog.vpnhide.debug.debugCaptureInfo
import kotlinx.coroutines.test.runTest
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** Core's logcat session wire (`fixtures/core-rpc/logcat-session-status.json`, written by core-rpc). */
class CoreLogcatRecordingTest {
    @After fun detach() = CoreRoot.setTransport(null)

    @Test
    fun `parses every session shape core writes`() {
        val all = JSONArray(File("../../fixtures/core-rpc/logcat-session-status.json").readText())
        val sessions = List(all.length()) { parseCoreLogcatSession(all.getJSONObject(it)) }

        val running = sessions[0]
        assertEquals(42L, running.id)
        assertTrue(running.recording)
        assertNull(running.autoStop)
        assertEquals(64L shl 20, running.maxBytes)
        assertEquals(1_800_000L, running.maxDurationMs)
        assertNull(running.file)

        val capped = sessions[1]
        assertFalse(capped.recording)
        assertEquals(LogcatAutoStop.SizeCap, capped.autoStop)
        assertTrue(capped.truncated)
        assertEquals("logcat-session-43.txt", capped.file?.name)

        assertEquals(LogcatAutoStop.LogcatExited, sessions[2].autoStop)
        assertEquals(1, sessions[2].logcatExit)

        // Core's debug lease and delivery state.
        assertEquals(LogcatDebugRestore.Held, running.debug?.restore)
        assertTrue(running.debug!!.forced)
        assertFalse(running.undelivered)
        assertEquals(LogcatDebugRestore.UserChoice, capped.debug?.restore)
        assertTrue(capped.undelivered)
        assertTrue(sessions[2].delivered)
        assertNull(sessions[2].debug)
        assertFalse(sessions[2].undelivered)

        val restarted = sessions[3]
        assertEquals(LogcatAutoStop.CoreRestarted, restarted.autoStop)
        assertEquals(LogcatDebugRestore.Restored, restarted.debug?.restore)
        assertTrue(restarted.undelivered)
    }

    @Test
    fun `the zip's debug capture comes from core's lease`() {
        val all = JSONArray(File("../../fixtures/core-rpc/logcat-session-status.json").readText())
        val sessions = List(all.length()) { parseCoreLogcatSession(all.getJSONObject(it)) }
        assertEquals(DebugCaptureInfo(true, 0, null, "core lease, restore=held"), sessions[0].debugCaptureInfo())
        assertEquals(DebugCaptureInfo(true, 0, 0, "core lease, restore=user_choice"), sessions[1].debugCaptureInfo())
        assertNull(sessions[2].debugCaptureInfo())
        val pending = sessions[3].copy(debug = sessions[3].debug!!.copy(previousDebug = true, restore = LogcatDebugRestore.Pending))
        assertEquals(DebugCaptureInfo(false, 0, -1, "core lease, restore=pending"), pending.debugCaptureInfo())
    }

    @Test
    fun `replies map to ok, busy and failure`() {
        assertEquals(CoreLogcatReply.Ok(null), parseCoreLogcatReply("""{"status":"ok","session":null}"""))
        assertEquals(CoreLogcatReply.Busy(7), parseCoreLogcatReply("""{"status":"busy","detail":"x","running":7}"""))
        assertEquals(
            CoreLogcatReply.Failed("disabled", "off"),
            parseCoreLogcatReply("""{"status":"disabled","detail":"off"}"""),
        )
        assertEquals("malformed", (parseCoreLogcatReply("""{"status":"ok"} trailing""") as CoreLogcatReply.Failed).status)
    }

    @Test
    fun `every action names the action and session only`() =
        runTest {
            val sent = mutableListOf<JSONObject>()
            CoreRoot.setTransport(
                object : CoreRootTransport {
                    override suspend fun collectDiagnostics(requestJson: String): String = ""

                    override suspend fun rootEffect(
                        op: Long,
                        effectJson: String,
                    ): String = ""

                    override suspend fun logcatRecording(requestJson: String): String {
                        sent += JSONObject(requestJson)
                        return """{"status":"unknown_session","detail":"none"}"""
                    }
                },
            )
            CoreRoot.startLogcat(5)
            CoreRoot.stopLogcat(5)
            assertEquals(CoreLogcatReply.Failed("unknown_session", "none"), CoreRoot.logcatStatus())
            CoreRoot.ackLogcat(5)
            CoreRoot.discardLogcat(5)
            assertEquals(listOf("start", "stop", "status", "ack", "discard"), sent.map { it.getString("action") })
            assertEquals(5L, sent[0].getLong("session"))
            assertEquals(listOf(5L, 5L), sent.drop(3).map { it.getLong("session") })
            sent.forEach { assertEquals(setOf("action", "session"), it.keys().asSequence().toSet()) }
        }

    @Test
    fun `an unattached core is unreachable`() =
        runTest {
            assertEquals("unreachable", (CoreRoot.startLogcat(1) as CoreLogcatReply.Failed).status)
        }
}
