package dev.okhsunrog.vpnhide

import org.junit.Assert.*
import org.junit.Test

class AgentUiSessionTest {
    @Test
    fun `every advertised scenario starts without needing device state`() {
        val session = AgentUiSession()
        assertEquals(agentUiScenarioCatalog.size, agentUiScenarioCatalog.map { it.id }.toSet().size)
        for (spec in agentUiScenarioCatalog) {
            val state = session.start(AgentUiStartArgs(spec.id))
            assertEquals(spec.id, state.scenario)
            assertNull(state.composedRevision)
            assertEquals(spec.id == "help", state.helpOpen)
        }
    }

    @Test
    fun `invalid requests leave current preview intact`() {
        val session = AgentUiSession()
        val before = session.start(AgentUiStartArgs("kpm-runtime-missing"))
        assertThrows(IllegalArgumentException::class.java) { session.start(AgentUiStartArgs("typo")) }
        assertThrows(IllegalArgumentException::class.java) {
            session.start(AgentUiStartArgs("targets-empty", "first-setup"))
        }
        for (bad in listOf("../secret", "https://example.com", "", "kpm-install\n", "kpm-install#" + "a".repeat(512))) {
            assertThrows(IllegalArgumentException::class.java) {
                session.navigate(AgentUiNavigationArgs(before.sessionId, bad))
            }
        }
        assertEquals(before, session.state.value)
    }

    @Test
    fun `repeated anchor navigation creates new composition and back restores scenario`() {
        val session = AgentUiSession()
        val id = session.start(AgentUiStartArgs("kpm-runtime-missing")).sessionId
        val args = AgentUiNavigationArgs(id, "kpm-install#где-скачать")
        val first = session.navigate(args)
        session.observe(id, first.revision, destination = args.destination)
        assertEquals(first.revision, session.state.value.composedRevision)
        assertEquals(args.destination, session.state.value.observedDestination)
        val second = session.navigate(args)
        assertTrue(second.revision > first.revision)
        assertNull(second.composedRevision)
        session.observe(id, first.revision, action = "stale")
        assertNull(session.state.value.lastAction)
        val back = session.back(id)
        assertEquals("kpm-runtime-missing", back.scenario)
        assertFalse(back.helpOpen)
        assertNull(session.back(id).scenario)
    }

    @Test
    fun `old clients and late UI callbacks cannot change replacement or stopped sessions`() {
        val session = AgentUiSession()
        val old = session.start(AgentUiStartArgs("kpm-runtime-missing"))
        val current = session.start(AgentUiStartArgs("help", "first-setup"))
        assertNotEquals(old.sessionId, current.sessionId)
        assertThrows(IllegalArgumentException::class.java) { session.stop(old.sessionId) }
        assertThrows(IllegalArgumentException::class.java) { session.back(old.sessionId) }
        session.observe(old.sessionId, old.revision, action = "late")
        assertEquals(current, session.state.value)
        session.clear()
        session.observe(current.sessionId, current.revision, action = "late")
        assertEquals(AgentUiState(), session.state.value)
        assertTrue(session.start(AgentUiStartArgs("help")).sessionId > current.sessionId)
    }

    @Test
    fun `disable clears session and prevents late requests from recreating preview`() {
        val session = AgentUiSession()
        val old = session.start(AgentUiStartArgs("help"))
        session.setEnabled(false)
        assertThrows(IllegalStateException::class.java) { session.start(AgentUiStartArgs("help")) }
        session.interact(old.sessionId, old.revision) { fail("Stale UI callback ran") }
        assertNull(session.state.value.scenario)
        session.setEnabled(true)
        assertNull(session.state.value.scenario)
        assertTrue(session.start(AgentUiStartArgs("help")).sessionId > old.sessionId)
    }

    @Test
    fun `recorded actions do not navigate or end the preview`() {
        val session = AgentUiSession()
        val state = session.start(AgentUiStartArgs("ports-inactive"))
        session.observe(state.sessionId, state.revision, action = "vpnhide://diagnostics")
        assertEquals("vpnhide://diagnostics", session.state.value.lastAction)
        assertEquals(state.scenario, session.state.value.scenario)
        assertFalse(session.state.value.helpOpen)
    }
}
