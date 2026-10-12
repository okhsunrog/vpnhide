package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds

/** The bit this hook occupies in a wire mask (protocol §5). The single shift. */
internal val HookIds.Hook.bit: Long get() = 1L shl id

/** Whether this wire mask (a packed Long) contains [hook]. */
internal fun Long.hasHook(hook: HookIds.Hook): Boolean = this and hook.bit != 0L

/** Fold a set of hooks back into a packed wire mask (protocol §5). */
internal fun Iterable<HookIds.Hook>.toHookMask(): Long = fold(0L) { acc, hook -> acc or hook.bit }

/** The hooks a raw wire mask selects, in registry order. */
internal fun hooksInMask(mask: Long): Set<HookIds.Hook> = HookIds.Hook.entries.filterTo(linkedSetOf()) { mask.hasHook(it) }

/**
 * Hooks each backend owns, derived **once** from the generated per-backend masks.
 * A backend acts only on the hooks it owns and ignores foreign bits (protocol §5).
 */
internal val KERNEL_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.KERNEL_HOOK_MASK.toLong())
internal val KMOD_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.KMOD_HOOK_MASK.toLong())
internal val BUILTIN_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.BUILTIN_HOOK_MASK.toLong())
internal val KPM_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.KPM_HOOK_MASK.toLong())
internal val ZYGISK_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.ZYGISK_HOOK_MASK.toLong())
internal val LSPOSED_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.LSPOSED_HOOK_MASK.toLong())

// ── wire status codes → registry enums ──────────────────────────────────────
// Protocol.Status keeps its fields as raw Longs so the wire stays decoupled from
// the registry; these are the one place those codes resolve back to the enums.

private val STATUS_ERROR_BY_CODE: Map<Long, HookIds.StatusError> = HookIds.StatusError.entries.associateBy { it.code.toLong() }
private val BACKEND_BY_ID: Map<Long, HookIds.Backend> = HookIds.Backend.entries.associateBy { it.id.toLong() }

/** Resolve the wire error code (protocol §5.1) to its [HookIds.StatusError], or null if unknown. */
internal val Protocol.Status.statusError: HookIds.StatusError? get() = STATUS_ERROR_BY_CODE[error]

/** Resolve the wire backend id (protocol §4.3) to its [HookIds.Backend], or null if unknown. */
internal val Protocol.Status.backendId: HookIds.Backend? get() = BACKEND_BY_ID[backend]

/** OK and PARTIAL_HOOKS both mean the backend loaded and is serving at least some
 * owned hooks — the states that count as a live/active backend. */
internal val HookIds.StatusError.indicatesActive: Boolean
    get() = this == HookIds.StatusError.OK || this == HookIds.StatusError.PARTIAL_HOOKS
