package dev.okhsunrog.vpnhide

internal data class CanonicalEditorState(
    val base: CanonicalConfig? = null,
    val draft: EditorDraft<CanonicalEdit> = EditorDraft(),
) {
    val current: CanonicalConfig? get() =
        base?.let {
            draft.edits.values.fold(
                it,
            ) { config, edit -> applyCanonicalEdit(config, edit.value) }
        }
    val dirty: Boolean get() = draft.edits.isNotEmpty()
}

internal fun changeCanonicalEditor(
    state: CanonicalEditorState,
    before: CanonicalConfig,
    after: CanonicalConfig,
    savePending: Boolean = false,
): CanonicalEditorState {
    val base = state.base ?: return state
    var draft = state.draft
    canonicalEdits(before, after).forEach { edit ->
        val field = canonicalEditFields(edit).single()
        draft =
            if (!savePending && applyCanonicalEdit(base, edit) == base) {
                draft.copy(edits = draft.edits - field)
            } else {
                editDraft(draft, field, canonicalEditSnapshot(edit))
            }
    }
    return state.copy(draft = draft)
}

internal fun rebaseCanonicalEditor(
    state: CanonicalEditorState,
    confirmed: CanonicalConfig,
): CanonicalEditorState = state.copy(base = canonicalConfigSnapshot(confirmed), draft = rebaseDraft(state.draft, emptyMap()))

internal fun acknowledgeCanonicalEditor(
    state: CanonicalEditorState,
    submitted: Map<ConfigField, DraftValue<CanonicalEdit>>,
): CanonicalEditorState = state.copy(draft = acknowledgeDraft(state.draft, submitted, emptyMap()))

internal fun discardCanonicalEditor(state: CanonicalEditorState): CanonicalEditorState =
    state.copy(draft = EditorDraft(revision = state.draft.revision + 1))

internal data class CanonicalEditorSubmission(
    val operationId: Long,
    val edits: Map<ConfigField, DraftValue<CanonicalEdit>>,
)
