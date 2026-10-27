package dev.okhsunrog.vpnhide

internal data class DraftValue<T>(
    val value: T,
    val revision: Long,
)

/** Values are typed by the editor; normalized paths keep role/hook edits in one conflict domain. */
internal data class EditorDraft<T>(
    val base: Map<ConfigField, T> = emptyMap(),
    val edits: Map<ConfigField, DraftValue<T>> = emptyMap(),
    val revision: Long = 0,
)

internal fun <T> editDraft(
    draft: EditorDraft<T>,
    field: ConfigField,
    value: T,
): EditorDraft<T> {
    val revision = draft.revision + 1
    return draft.copy(edits = draft.edits + (field to DraftValue(value, revision)), revision = revision)
}

internal fun <T> rebaseDraft(
    draft: EditorDraft<T>,
    confirmed: Map<ConfigField, T>,
): EditorDraft<T> = draft.copy(base = confirmed.toMap())

/** Handles both immediate persistence and later reconciliation without dropping newer edits. */
internal fun <T> acknowledgeDraft(
    draft: EditorDraft<T>,
    submitted: Map<ConfigField, DraftValue<T>>,
    confirmed: Map<ConfigField, T>,
): EditorDraft<T> =
    draft.copy(
        base = confirmed.toMap(),
        edits = draft.edits.filter { (field, edit) -> submitted[field]?.revision != edit.revision },
    )

internal fun <T> draftValues(draft: EditorDraft<T>): Map<ConfigField, T> = draft.base + draft.edits.mapValues { it.value.value }
