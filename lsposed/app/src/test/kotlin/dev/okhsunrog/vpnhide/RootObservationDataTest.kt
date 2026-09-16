package dev.okhsunrog.vpnhide

import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RootObservationDataTest {
    @Test
    fun `retry after first root failure also wakes failed inventory and projections`() {
        val cold = ObservationState<RootSnapshot>()
        val initial = reduceObservation(cold, ObservationEvent.Ensure(1)).state
        assertFalse(rootObservationInvalidatesDependents(cold, initial))
        assertFalse(rootObservationInvalidatesInventory(cold, initial))
        val failed = reduceObservation(initial, ObservationEvent.Failed(1, TransitionFailure.ReadFailed, 2)).state
        val retry = reduceObservation(failed, ObservationEvent.Refresh(3)).state
        assertTrue(rootObservationInvalidatesDependents(failed, retry))
        assertTrue(rootObservationInvalidatesInventory(failed, retry))
    }

    @Test
    fun `inventory invalidates on changed publication but not routine root refresh`() {
        val loaded = loaded()
        val refreshing = reduceObservation(loaded, ObservationEvent.Refresh(3)).state
        assertFalse(rootObservationInvalidatesInventory(loaded, refreshing))
        val snapshot = RootSnapshot(mapOf("pm_users" to "work"), observationId = 2)
        val published = reduceObservation(refreshing, ObservationEvent.Loaded(2, snapshot, 4)).state
        assertTrue(rootObservationInvalidatesInventory(refreshing, published))
        assertFalse(rootObservationInvalidatesInventory(published, published))
    }

    @Test
    fun `root refresh invalidates projections before a new snapshot arrives`() {
        val loaded = loaded()
        val refreshing = reduceObservation(loaded, ObservationEvent.Refresh(3)).state
        assertTrue(rootObservationInvalidatesDependents(loaded, refreshing))
        val invalidated = reduceObservation(refreshing, ObservationEvent.Invalidate(4)).state
        assertTrue(rootObservationInvalidatesDependents(refreshing, invalidated))
        val successor = reduceObservation(invalidated, ObservationEvent.Loaded(2, RootSnapshot(emptyMap()), 5)).state
        assertFalse(rootObservationInvalidatesDependents(invalidated, successor))
    }

    @Test
    fun `inventory consumers ignore unrelated config and telemetry changes`() {
        val original = RootSnapshot(mapOf("pm_packages" to "one", "pm_users" to "user0", "canonical_config" to "old"))
        assertFalse(rootInventoryChanged(null, original))
        assertFalse(rootInventoryChanged(original, original.copy(sections = original.sections + ("canonical_config" to "new"))))
        assertTrue(rootInventoryChanged(original, original.copy(sections = original.sections + ("pm_packages" to "two"))))
        assertTrue(rootInventoryChanged(original, original.copy(sections = original.sections + ("pm_users" to "work"))))
    }

    @Test
    fun `flow projections expose one committed payload without separate publishing jobs`() {
        val source = MutableStateFlow(loaded())
        val retained = ProjectedStateFlow(source) { it.lastGood?.value }
        val current = ProjectedStateFlow(source, ::currentObservationValue)
        val loading = ProjectedStateFlow(source) { it.active != null }
        assertEquals(retained.value, current.value)
        source.value = reduceObservation(source.value, ObservationEvent.Refresh(3)).state
        assertTrue(loading.value)
        assertNull(current.value)
        assertEquals(1L, retained.value?.observationId)
        source.value = reduceObservation(source.value, ObservationEvent.Failed(2, TransitionFailure.ReadFailed, 4)).state
        assertFalse(loading.value)
        assertNull(current.value)
        assertEquals(1L, retained.value?.observationId)
    }

    @Test
    fun `cold invalidation defers loading and resource recovery may remain failed without retrying`() {
        val deferred = reduceObservation(ObservationState<String>(), ObservationEvent.Invalidate(1, start = false))
        assertFalse(deferred.state.attempted)
        assertTrue(deferred.effects.isEmpty())
        val active = reduceObservation(deferred.state, ObservationEvent.Ensure(2)).state
        val expired = reduceObservation(active, ObservationEvent.Failed(1, TransitionFailure.DeadlineExceeded, 3, false)).state
        val recovered = reduceObservation(expired, ObservationEvent.ResourceRecovered(1, 4, retry = false))
        assertFalse(recovered.state.quarantined)
        assertNull(recovered.state.active)
        assertTrue(recovered.effects.isEmpty())
        assertEquals(TransitionFailure.DeadlineExceeded, recovered.state.error)
    }

    private fun loaded(): ObservationState<RootSnapshot> {
        val active = reduceObservation(ObservationState<RootSnapshot>(), ObservationEvent.Ensure(1)).state
        return reduceObservation(active, ObservationEvent.Loaded(1, RootSnapshot(emptyMap(), observationId = 1), 2)).state
    }
}
