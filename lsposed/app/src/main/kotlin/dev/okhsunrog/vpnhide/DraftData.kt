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

/** Local token ownership and confirmed storage are separate: release never acknowledges root cleanup. */
internal data class CaptureLoggingState(
    val userEnabled: Boolean,
    val tokens: Set<Long> = emptySet(),
    val revision: Long = 0,
    val confirmedEnabled: Boolean = userEnabled,
    // An interrupted capture can leave storage different from the initial desired intent.
    val confirmedRevision: Long = if (confirmedEnabled == (userEnabled || tokens.isNotEmpty())) revision else -1,
) {
    val desiredEnabled: Boolean get() = userEnabled || tokens.isNotEmpty()
    val cleanupPending: Boolean get() = desiredEnabled != confirmedEnabled || revision != confirmedRevision
}

internal sealed interface CaptureLoggingEvent {
    data class Acquire(
        val id: Long,
    ) : CaptureLoggingEvent

    data class Release(
        val id: Long,
    ) : CaptureLoggingEvent

    data class UserChanged(
        val enabled: Boolean,
    ) : CaptureLoggingEvent

    data class Confirmed(
        val revision: Long,
        val enabled: Boolean,
    ) : CaptureLoggingEvent
}

internal fun reduceCaptureLogging(
    state: CaptureLoggingState,
    event: CaptureLoggingEvent,
): CaptureLoggingState =
    when (event) {
        is CaptureLoggingEvent.Acquire -> {
            if (event.id in state.tokens) state else state.copy(tokens = state.tokens + event.id, revision = state.revision + 1)
        }

        is CaptureLoggingEvent.Release -> {
            if (event.id !in state.tokens) state else state.copy(tokens = state.tokens - event.id, revision = state.revision + 1)
        }

        is CaptureLoggingEvent.UserChanged -> {
            if (state.userEnabled == event.enabled) state else state.copy(userEnabled = event.enabled, revision = state.revision + 1)
        }

        is CaptureLoggingEvent.Confirmed -> {
            if (event.revision <= state.confirmedRevision || event.revision > state.revision) {
                state
            } else {
                state.copy(confirmedEnabled = event.enabled, confirmedRevision = event.revision)
            }
        }
    }
