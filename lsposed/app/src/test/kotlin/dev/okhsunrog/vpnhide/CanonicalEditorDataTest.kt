package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalEditorDataTest {
    private val base = CanonicalConfig()
    private val selected = base.copy(debugSwitch = true)

    @Test
    fun `rebase preserves UI edit and merges unrelated confirmed fields`() {
        val draft = changeCanonicalEditor(CanonicalEditorState(base), base, selected)
        val rebased = rebaseCanonicalEditor(draft, base.copy(settings = base.settings.copy(autoHideVpnName = true)))
        assertTrue(requireNotNull(rebased.current).debugSwitch)
        assertTrue(requireNotNull(rebased.current).settings.autoHideVpnName)
        assertTrue(rebased.dirty)
        assertEquals(1, rebased.draft.edits.size)
    }

    @Test
    fun `reverting while save is in flight remains a newer UI intent`() {
        val draft = changeCanonicalEditor(CanonicalEditorState(base), base, selected)
        val reverted = changeCanonicalEditor(draft, selected, base, savePending = true)
        val saved = rebaseCanonicalEditor(reverted, selected)
        val acknowledged = acknowledgeCanonicalEditor(saved, draft.draft.edits)
        assertTrue(acknowledged.dirty)
        assertFalse(requireNotNull(acknowledged.current).debugSwitch)
    }

    @Test
    fun `discard and edit cannot reuse the revision of an old save`() {
        val draft = changeCanonicalEditor(CanonicalEditorState(base), base, selected)
        val replacement = changeCanonicalEditor(discardCanonicalEditor(draft), base, selected)
        val acknowledged = acknowledgeCanonicalEditor(rebaseCanonicalEditor(replacement, selected), draft.draft.edits)
        assertTrue(acknowledged.dirty)
        assertTrue(replacement.draft.revision > draft.draft.revision)
    }

    @Test
    fun `acknowledgement clears only submitted revisions and an idle revert is clean`() {
        val draft = changeCanonicalEditor(CanonicalEditorState(base), base, selected)
        assertFalse(changeCanonicalEditor(draft, selected, base).dirty)
        assertFalse(acknowledgeCanonicalEditor(rebaseCanonicalEditor(draft, selected), draft.draft.edits).dirty)
    }

    @Test
    fun `filesystem feature produces one field edit preserving other optional features`() {
        val before = base.copy(settings = base.settings.copy(optionalFeatures = setOf("future-feature")))
        val after = applyCanonicalEdit(before, CanonicalEdit.Toggle(CanonicalToggle.Filesystem, true))
        val edits = canonicalEdits(before, after)
        assertEquals(1, edits.size)
        assertEquals(after, applyCanonicalEdit(before, edits.single()))
        assertTrue("future-feature" in after.settings.optionalFeatures)
    }
}
