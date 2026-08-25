package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.NativeCheckSpec
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.verdict
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.statistics.parseProtocolStatusBlock

// The set-shaped view over the generated hook registry.
//
// The control/stats protocol packs hooks into a bitmask (docs/protocol.md §5):
// bit N == hook id N, carried as a Long in Protocol.Status.hooks and
// Protocol.Target.mask. That packing is a *wire* detail — it stays at the wire
// boundary. Everywhere else in the app we work with HookIds.Hook values and real
// Sets, so no call site reimplements `1 shl id` or `mask and bit` (they used to,
// six times over, in both Int and Long flavours).

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
internal val KPM_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.KPM_HOOK_MASK.toLong())
internal val ZYGISK_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.ZYGISK_HOOK_MASK.toLong())
internal val LSPOSED_HOOKS: Set<HookIds.Hook> = hooksInMask(HookIds.LSPOSED_HOOK_MASK.toLong())

/** The hooks owned by [backend] (its generated mask), or an empty set for an unknown id. */
internal fun ownedHooks(backend: HookIds.Backend): Set<HookIds.Hook> =
    when (backend) {
        HookIds.Backend.KMOD -> KERNEL_HOOKS + KMOD_HOOKS
        HookIds.Backend.KPM -> KERNEL_HOOKS + KPM_HOOKS
        HookIds.Backend.ZYGISK -> ZYGISK_HOOKS
        HookIds.Backend.LSPOSED -> LSPOSED_HOOKS
    }

/**
 * The hooks the active native backend owns: Zygisk owns its libc hooks; every
 * kernel backend — and the null/none case — owns the kernel hooks. Single source
 * so the tile verdict, the unowned-leak count, and [buildDiagnosticReport] all
 * scope "owned vectors" the same way.
 */
internal fun ownedNativeHooks(
    id: NativeBackendId?,
    installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
): Set<HookIds.Hook> =
    when (id) {
        NativeBackendId.Kmod -> {
            KERNEL_HOOKS + installedOptionalHooks.intersect(KMOD_HOOKS)
        }

        NativeBackendId.Kpm -> {
            KERNEL_HOOKS + installedOptionalHooks.intersect(KPM_HOOKS)
        }

        NativeBackendId.Zygisk -> {
            ZYGISK_HOOKS - HookIds.Hook.FILESYSTEM_IFACE_PATHS +
                installedOptionalHooks.intersect(setOf(HookIds.Hook.FILESYSTEM_IFACE_PATHS))
        }

        null -> {
            KERNEL_HOOKS
        }
    }

/**
 * Hooks the active kernel backend should have installed but did not — the gap
 * between what its family owns and what its status mask reports (protocol §5.1,
 * the same gap that makes it report `PARTIAL_HOOKS`).
 *
 * A kernel backend resolves each target by name at load time, so one that a
 * vendor kernel renamed (or dropped from kallsyms) is simply skipped and the
 * vectors behind it stay visible. Naming those hooks is the difference between
 * "leak, reinstall the module" and "your kernel does not expose sock_ioctl".
 *
 * Empty for Zygisk: its heartbeat mask is per-process and its optional hooks are
 * tracked separately, so a diff would report noise, not a defect.
 */
internal fun missingBackendHooks(
    id: NativeBackendId?,
    reportedHooks: Set<HookIds.Hook>,
): Set<HookIds.Hook> =
    when (id) {
        NativeBackendId.Kmod, NativeBackendId.Kpm -> {
            // An empty report means "no status read", not "nothing installed".
            if (reportedHooks.isEmpty()) emptySet() else KERNEL_HOOKS - reportedHooks
        }

        NativeBackendId.Zygisk, null -> {
            emptySet()
        }
    }

/** Decode a backend's installed-hook wire mask into the typed registry set. */
internal fun installedHooks(statusRaw: String): Set<HookIds.Hook> = parseProtocolStatusBlock(statusRaw)?.hooks?.let(::hooksInMask).orEmpty()

/** Hooks expected in a live status snapshot. Backend-specific hooks are optional and
 * become part of the expectation only when that boot actually installed them. */
internal fun expectedInstalledHooks(
    backend: HookIds.Backend,
    installed: Set<HookIds.Hook>,
): Set<HookIds.Hook> =
    when (backend) {
        HookIds.Backend.KMOD -> KERNEL_HOOKS + installed.intersect(KMOD_HOOKS)
        HookIds.Backend.KPM -> KERNEL_HOOKS + installed.intersect(KPM_HOOKS)
        else -> ownedHooks(backend)
    }

/** Whether any hook that should cover this vector is in [hooks]. */
internal fun NativeCheckSpec.coveredBy(hooks: Set<HookIds.Hook>): Boolean = expectedHooks.any { it in hooks }

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
