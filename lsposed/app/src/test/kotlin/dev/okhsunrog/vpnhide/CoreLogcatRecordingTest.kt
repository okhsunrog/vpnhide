package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreLogcatReply
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.core.LogcatAutoStop
import dev.okhsunrog.vpnhide.core.LogcatDebugRestore
import dev.okhsunrog.vpnhide.core.coreLogcatReply
import dev.okhsunrog.vpnhide.core.coreLogcatSession
import dev.okhsunrog.vpnhide.debug.DebugCaptureInfo
import dev.okhsunrog.vpnhide.debug.debugCaptureInfo
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreLogcatAutoStopReason
import uniffi.vpnhide_core_ffi.CoreLogcatDebugLease
import uniffi.vpnhide_core_ffi.CoreLogcatDebugRestore
import uniffi.vpnhide_core_ffi.CoreLogcatError
import uniffi.vpnhide_core_ffi.CoreLogcatRequest
import uniffi.vpnhide_core_ffi.CoreLogcatSessionState
import uniffi.vpnhide_core_ffi.CoreLogcatSessionStatus
import uniffi.vpnhide_core_ffi.LogcatReply

class CoreLogcatRecordingTest {
    @After fun detach() = CoreRoot.setTransport(null)

    @Test
    fun `typed session retains auto stop delivery and lease evidence`() {
        val native =
            CoreLogcatSessionStatus(
                42u,
                CoreLogcatSessionState.Recording,
                1u,
                20u,
                32u,
                false,
                64uL shl 20,
                1800000u,
                null,
                null,
                false,
                CoreLogcatDebugLease(false, false, CoreLogcatDebugRestore.HELD),
            )
        val running = coreLogcatSession(native)
        assertTrue(running.recording)
        assertFalse(running.undelivered)
        assertEquals(LogcatDebugRestore.Held, running.debug!!.restore)
        assertEquals(DebugCaptureInfo(true, 0, null, "core lease, restore=held"), running.debugCaptureInfo())
        native.state = CoreLogcatSessionState.AutoStopped(CoreLogcatAutoStopReason.CORE_RESTARTED)
        native.file = "/x/logcat-42.txt"
        val stopped = coreLogcatSession(native)
        assertEquals(LogcatAutoStop.CoreRestarted, stopped.autoStop)
        assertTrue(stopped.undelivered)
        assertEquals("logcat-42.txt", stopped.file!!.name)
        native.debug!!.restore = CoreLogcatDebugRestore.USER_CHOICE
        assertEquals(LogcatDebugRestore.UserChoice, coreLogcatSession(native).debug!!.restore)
        native.delivered = true
        assertFalse(coreLogcatSession(native).undelivered)
    }

    @Test
    fun `actions and refusals use generated vocabulary`() =
        runTest {
            val sent = mutableListOf<CoreLogcatRequest>()
            CoreRoot.setTransport(
                object : TestCoreRootTransport() {
                    override suspend fun logcatRecording(request: CoreLogcatRequest): LogcatReply {
                        sent += request
                        return LogcatReply.Refused(CoreLogcatError.UnknownSession)
                    }
                },
            )
            CoreRoot.startLogcat(5)
            CoreRoot.stopLogcat(5)
            assertEquals(CoreLogcatReply.Failed("unknown_session", ""), CoreRoot.logcatStatus())
            CoreRoot.ackLogcat(5)
            CoreRoot.discardLogcat(5)
            assertEquals(
                listOf(
                    CoreLogcatRequest.Start(5u),
                    CoreLogcatRequest.Stop(5u),
                    CoreLogcatRequest.Status,
                    CoreLogcatRequest.Ack(5u),
                    CoreLogcatRequest.Discard(5u),
                ),
                sent,
            )
            assertEquals(CoreLogcatReply.Busy(7), coreLogcatReply(LogcatReply.Refused(CoreLogcatError.Busy(7u))))
        }
}
