package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DraftDataTest {
    private val java = ConfigField(listOf("apps", "one.app", "java"))
    private val native = ConfigField(listOf("apps", "one.app", "native"))

    @Test
    fun `initial interrupted-capture discrepancy can acknowledge cleanup without a new UI edit`() {
        val initial = CaptureLoggingState(userEnabled = false, confirmedEnabled = true)
        assertTrue(initial.cleanupPending)
        val restored = reduceCaptureLogging(initial, CaptureLoggingEvent.Confirmed(initial.revision, false))
        assertFalse(restored.cleanupPending)
        assertEquals(restored, reduceCaptureLogging(restored, CaptureLoggingEvent.Confirmed(initial.revision, true)))
    }

    @Test
    fun `late persistence acknowledgement keeps later UI edit even if its value returned to submitted value`() {
        val original = editDraft(EditorDraft<Boolean>(), java, true)
        val submitted = original.edits.toMap()
        val later = editDraft(editDraft(original, java, false), java, true)
        val acknowledged = acknowledgeDraft(later, submitted, mapOf(java to true, native to true))
        assertEquals(later.edits, acknowledged.edits)
        assertEquals(mapOf(java to true, native to true), draftValues(acknowledged))
    }

    @Test
    fun `rebase merges untouched fields and acknowledged save clears only submitted revision`() {
        val draft = editDraft(EditorDraft(mapOf(java to false, native to false)), java, true)
        val rebased = rebaseDraft(draft, mapOf(java to false, native to true))
        assertEquals(mapOf(java to true, native to true), draftValues(rebased))
        val saved = acknowledgeDraft(rebased, draft.edits, mapOf(java to true, native to true))
        assertTrue(saved.edits.isEmpty())
    }

    @Test
    fun `package field paths do not confuse dotted names and ancestor deletion conflicts`() {
        assertTrue(configFieldsOverlap(ConfigField(listOf("apps", "one.app")), java))
        assertFalse(configFieldsOverlap(ConfigField(listOf("apps", "one")), java))
        assertFalse(configFieldsOverlap(java, native))
    }

    @Test
    fun `overlapping capture release preserves user intent and records deferred cleanup`() {
        var state = CaptureLoggingState(userEnabled = false)
        state = reduceCaptureLogging(state, CaptureLoggingEvent.Acquire(1))
        state = reduceCaptureLogging(state, CaptureLoggingEvent.Acquire(2))
        state = reduceCaptureLogging(state, CaptureLoggingEvent.Confirmed(state.revision, true))
        state = reduceCaptureLogging(state, CaptureLoggingEvent.Release(1))
        assertTrue(state.desiredEnabled)
        state = reduceCaptureLogging(state, CaptureLoggingEvent.UserChanged(true))
        state = reduceCaptureLogging(state, CaptureLoggingEvent.UserChanged(false))
        state = reduceCaptureLogging(state, CaptureLoggingEvent.Release(2))
        assertFalse(state.desiredEnabled)
        assertTrue(state.confirmedEnabled)
        assertTrue(state.cleanupPending)
        assertEquals(state, reduceCaptureLogging(state, CaptureLoggingEvent.Release(2)))
        state = reduceCaptureLogging(state, CaptureLoggingEvent.Confirmed(state.revision, false))
        assertFalse(state.cleanupPending)
        val late = reduceCaptureLogging(state, CaptureLoggingEvent.Confirmed(2, true))
        assertEquals(state, late)
    }
}
