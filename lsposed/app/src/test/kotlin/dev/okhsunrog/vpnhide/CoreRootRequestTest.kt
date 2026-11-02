package dev.okhsunrog.vpnhide

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.vpnhide_core_ffi.CoreDiagnosticException
import uniffi.vpnhide_core_ffi.CoreDiagnosticRefusal
import uniffi.vpnhide_core_ffi.CoreException

class CoreRootRequestTest {
    @Test
    fun `diagnostic transport withdraws current publication through its connection owner`() =
        runTest {
            val state = CoreStateView(7, 1, RootCanonicalRead.Missing, emptyList())
            var owner = acceptCorePublication(CorePublicationData(), 3, 4, state)
            val failure = CoreException.Transport(3uL, "reply lost")
            try {
                coreRootRequest<Unit>(5, { request, error ->
                    assertSame(failure, error)
                    owner = failCorePublication(owner, 3, request, false)
                }) { throw CoreDiagnosticException.Transport(failure) }
                error("expected transport failure")
            } catch (error: CoreException.Transport) {
                assertSame(failure, error)
            }
            assertFalse(owner.update.current)
            assertSame(state, owner.update.state)
        }

    @Test
    fun `late diagnostic failure cannot withdraw a newer connection`() =
        runTest {
            val state = CoreStateView(8, 1, RootCanonicalRead.Missing, emptyList())
            var owner = acceptCorePublication(CorePublicationData(), 4, 6, state)
            try {
                coreRootRequest<Unit>(7, { request, _ -> owner = failCorePublication(owner, 3, request, false) }) {
                    throw CoreDiagnosticException.Transport(CoreException.Unavailable(3uL, "old connection"))
                }
                error("expected failure")
            } catch (_: CoreException.Unavailable) {
                assertTrue(owner.update.current)
                assertSame(state, owner.update.state)
            }
        }

    @Test
    fun `admission refusal and waiter cancellation do not poison the connection`() =
        runTest {
            val refusal = CoreDiagnosticException.Refused(CoreDiagnosticRefusal.BUSY)
            try {
                coreRootRequest<Unit>(1, { _, _ -> error("refusal is not transport failure") }) { throw refusal }
                error("expected refusal")
            } catch (error: CoreDiagnosticException.Refused) {
                assertSame(refusal, error)
            }
            val cancellation = CancellationException("waiter detached")
            try {
                coreRootRequest<Unit>(2, { _, _ -> error("waiter does not own connection") }) { throw cancellation }
                error("expected cancellation")
            } catch (error: CancellationException) {
                assertSame(cancellation, error)
            }
        }

    @Test
    fun `nested protocol mismatch keeps the explicit skew state and retained source`() =
        runTest {
            val state = CoreStateView(8, 1, RootCanonicalRead.Missing, emptyList())
            var owner = acceptCorePublication(CorePublicationData(), 4, 6, state)
            val failure = CoreException.ProtocolSkew(4uL, 17u, 18u)
            try {
                coreRootRequest<Unit>(7, { request, error ->
                    owner = failCorePublication(owner, 4, request, error is CoreException.ProtocolSkew)
                }) { throw CoreDiagnosticException.Transport(failure) }
                error("expected protocol mismatch")
            } catch (error: CoreException.ProtocolSkew) {
                assertSame(failure, error)
            }
            assertFalse(owner.update.current)
            assertTrue(owner.update.protocolSkew)
            assertSame(state, owner.update.state)
        }
}
