package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.core.CoreRootEffect
import uniffi.vpnhide_core_ffi.CoreCommand
import uniffi.vpnhide_core_ffi.CoreEdit

/** Resolve app intent to one durable command; composition and recovery stay in core. */
internal fun coreCommand(
    base: Long,
    mutation: CanonicalMutation,
    intent: PreparedConfigIntent,
): CoreCommand? {
    require(mutation.coupledEffects.size <= 1) { "A user action maps to one core command" }
    val revision = base.toULong()
    return when (val effect = mutation.coupledEffects.singleOrNull()) {
        is CoreRootEffect.SetSuperkey -> {
            require(intent.edits.all { it is CanonicalEdit.Toggle && it.field == CanonicalToggle.RememberSuperkey })
            CoreCommand.SetSuperkey(revision, intent.candidate.settings.rememberSuperkey, effect.superkey)
        }

        CoreRootEffect.ClearSuperkey -> {
            require(intent.edits.all { it is CanonicalEdit.Toggle && it.field == CanonicalToggle.RememberSuperkey })
            CoreCommand.ClearSuperkey(revision)
        }

        CoreRootEffect.FullReset -> {
            require(mutation.removesCanonical && intent.edits.isEmpty())
            CoreCommand.FullReset
        }

        CoreRootEffect.DeleteLegacyConfig -> {
            if (intent.edits.isEmpty()) CoreCommand.CleanupLegacy else CoreCommand.Import(revision, intent.candidate.toCore(), true)
        }

        null -> {
            when {
                intent.edits.any { it is CanonicalEdit.Replace } -> CoreCommand.Import(revision, intent.candidate.toCore(), false)
                intent.edits.isNotEmpty() -> CoreCommand.Edit(revision, coreEdit(intent.edits))
                mutation.forceActivation -> CoreCommand.Edit(revision, CoreEdit.RepairActivation)
                else -> null
            }
        }
    }
}
