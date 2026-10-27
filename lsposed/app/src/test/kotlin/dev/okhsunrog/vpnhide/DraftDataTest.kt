package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DraftDataTest {
    private val java = ConfigField(listOf("apps", "one.app", "java"))
    private val native = ConfigField(listOf("apps", "one.app", "native"))

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
}
