package dev.okhsunrog.vpnhide

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicLong

/** Activity ViewModel only: no process-death persistence and no Activity/Context retained. */
internal class CanonicalEditorViewModel : ViewModel() {
    private val id = nextId.getAndIncrement()
    var state by mutableStateOf(CanonicalEditorState())
        private set
    var saving by mutableStateOf(false)
        private set
    var result by mutableStateOf<CanonicalWriteResult?>(null)
        private set
    private var pendingAck: CanonicalEditorSubmission? = null

    init {
        viewModelScope.launch {
            CanonicalConfigRepository.state.collect { view ->
                view.confirmed?.let { state = rebaseCanonicalEditor(state, it) }
                acknowledgeRecovery(view.recoveredResult)
            }
        }
    }

    fun change(
        before: CanonicalConfig,
        after: CanonicalConfig,
    ) {
        state = changeCanonicalEditor(state, before, after, savePending = saving || pendingAck != null)
        register()
    }

    fun discard() {
        state = discardCanonicalEditor(state)
        register()
    }

    fun save(
        activation: CanonicalActivation = CanonicalActivation(),
        transform: (CanonicalConfig) -> CanonicalConfig = { it },
    ) {
        if (saving) return
        val submitted = state.draft.edits.toMap()
        saving = true
        result = null
        viewModelScope.launch {
            try {
                val write =
                    CanonicalConfigRepository.commit(
                        CanonicalMutation(
                            submitted.values.map {
                                it.value
                            },
                            activation = activation,
                            transform = transform,
                        ),
                    )
                CanonicalConfigRepository.state.value.confirmed
                    ?.let { state = rebaseCanonicalEditor(state, it) }
                result = write
                if (write.succeeded || write.operation?.phases?.get(ConfigPhase.Persist) == PhaseOutcome.Confirmed) {
                    state = acknowledgeCanonicalEditor(state, submitted)
                } else if (write.operation?.phases?.get(ConfigPhase.Persist) == PhaseOutcome.Unknown) {
                    pendingAck = CanonicalEditorSubmission(requireNotNull(write.operation).id, submitted)
                    acknowledgeRecovery(CanonicalConfigRepository.state.value.recoveredResult)
                }
                register()
            } finally {
                saving = false
            }
        }
    }

    private fun acknowledgeRecovery(recovered: ConfigOperationResult?) {
        val pending = pendingAck ?: return
        if (recovered?.id != pending.operationId) return
        if (recovered.phases[ConfigPhase.Persist] == PhaseOutcome.Confirmed) {
            state = acknowledgeCanonicalEditor(state, pending.edits)
            register()
        }
        pendingAck = null
        result = CanonicalWriteResult(if (recovered.failure == null) 0 else -1, "", recovered)
    }

    private fun register() = CanonicalConfigRepository.draftChanged(id, state.draft.edits.keys)

    override fun onCleared() {
        CanonicalConfigRepository.draftChanged(id, emptySet())
    }

    companion object {
        private val nextId = AtomicLong(1)
    }
}

@Composable
internal fun rememberCanonicalEditor(key: String): CanonicalEditorViewModel {
    val owner = LocalLifecycleOwner.current as ViewModelStoreOwner
    return ViewModelProvider(owner)[key, CanonicalEditorViewModel::class.java]
}
