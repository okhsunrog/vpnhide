package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ObservationDataTest {
    @Test
    fun `invalidation during a load discards obsolete result and coalesces successor`() {
        var state = loadedObservation()
        state = reduceObservation(state, ObservationEvent.Refresh(10)).state
        val obsolete = requireNotNull(state.active).id
        repeat(3) { state = reduceObservation(state, ObservationEvent.Invalidate(11)).state }
        assertEquals(obsolete, state.active.id)
        val next = reduceObservation(state, ObservationEvent.Loaded(obsolete, "obsolete", 12))
        assertEquals("original", next.state.lastGood?.value)
        assertEquals(1, next.effects.filterIsInstance<ObservationEffect.Load>().size)
        assertEquals(ObservationEffect.Finished(obsolete, ObservationCompletion.Superseded), next.effects.first())
        state =
            reduceObservation(
                next.state,
                ObservationEvent.Failed(requireNotNull(next.state.active).id, TransitionFailure.ReadFailed, 13),
            ).state
        assertEquals("original", state.lastGood?.value)
        assertEquals(TransitionFailure.ReadFailed, state.error)
        val late = reduceObservation(state, ObservationEvent.Loaded(obsolete, "late", 14))
        assertEquals(state, late.state)
        assertTrue(late.effects.isEmpty())
    }

    @Test
    fun `equivalent refresh joins but a fresh-since request waits for a successor`() {
        val loading = reduceObservation(ObservationState<String>(), ObservationEvent.Refresh(10)).state
        val joined = reduceObservation(loading, ObservationEvent.Refresh(11, notBefore = 10))
        assertEquals(listOf(ObservationEffect.Join(1)), joined.effects)
        val fresh = reduceObservation(loading, ObservationEvent.Refresh(12, notBefore = 12))
        assertEquals(1L, fresh.state.active?.id)
        assertTrue(fresh.state.generation > loading.generation)
        val successor = reduceObservation(fresh.state, ObservationEvent.Loaded(1, "too old", 13))
        assertEquals(13L, successor.state.active?.startedAt)
        assertEquals(null, successor.state.lastGood)
    }

    @Test
    fun `ensure cannot endlessly retry a failed load on recomposition`() {
        val loading = reduceObservation(ObservationState<String>(), ObservationEvent.Ensure(1)).state
        val failed = reduceObservation(loading, ObservationEvent.Failed(1, TransitionFailure.ReadFailed, 2)).state
        assertTrue(reduceObservation(failed, ObservationEvent.Ensure(3)).effects.isEmpty())
        assertFalse(reduceObservation(failed, ObservationEvent.Refresh(3)).effects.isEmpty())
    }

    @Test
    fun `unproven read cleanup quarantines resource until explicit recovery`() {
        val loading = reduceObservation(loadedObservation(), ObservationEvent.Refresh(10)).state
        val failed = reduceObservation(loading, ObservationEvent.Failed(2, TransitionFailure.DeadlineExceeded, 11, quiescent = false)).state
        assertTrue(failed.quarantined)
        assertEquals("original", failed.lastGood?.value)
        val refresh = reduceObservation(failed, ObservationEvent.Refresh(12))
        assertTrue(refresh.effects.single() is ObservationEffect.Unavailable)
        assertEquals(failed, reduceObservation(failed, ObservationEvent.ResourceRecovered(1, 13)).state)
        val recovered = reduceObservation(failed, ObservationEvent.ResourceRecovered(2, 13))
        assertFalse(recovered.state.quarantined)
        assertTrue(recovered.effects.single() is ObservationEffect.Load)
    }

    private fun loadedObservation(): ObservationState<String> {
        val loading = reduceObservation(ObservationState<String>(), ObservationEvent.Ensure(1)).state
        return reduceObservation(loading, ObservationEvent.Loaded(1, "original", 2)).state
    }
}
