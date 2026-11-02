package dev.okhsunrog.vpnhide
import dev.okhsunrog.vpnhide.diagnostics.coreInputsJoined
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreInputJoinDataTest {
    private val state = CoreStateView(7, 5, RootCanonicalRead.Available(CanonicalConfig()), emptyList())
    private val root = joinRootSnapshot(emptyMap(), CoreSessionUpdate(state, current = true, sequence = 12))
    private val accepted = ConfigCoordinatorView(coreCurrent = true, coreEpoch = 7, coreSequence = 12, desiredRevision = 5)

    @Test fun `root facts require accepted config revision epoch and publication`() {
        assertFalse(coreInputsJoined(root, accepted.copy(coreSequence = 11)))
        assertTrue(coreInputsJoined(root, accepted))
        assertTrue(coreInputsJoined(root, accepted.copy(coreSequence = 13)))
        assertFalse(coreInputsJoined(root, accepted.copy(coreEpoch = 8)))
        assertFalse(coreInputsJoined(root, accepted.copy(desiredRevision = 6)))
        assertFalse(coreInputsJoined(root, accepted.copy(coreCurrent = false)))
    }
}
