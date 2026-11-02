package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.diagnostics.NativeCheckSpec
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

/** The hooks owned by [backend] (its generated mask), or an empty set for an unknown id. */
internal fun ownedHooks(backend: HookIds.Backend): Set<HookIds.Hook> =
    when (backend) {
        HookIds.Backend.KMOD -> KERNEL_HOOKS + KMOD_HOOKS
        HookIds.Backend.BUILTIN -> KERNEL_HOOKS + BUILTIN_HOOKS
        HookIds.Backend.KPM -> KERNEL_HOOKS + KPM_HOOKS
        HookIds.Backend.ZYGISK -> ZYGISK_HOOKS
        HookIds.Backend.LSPOSED -> LSPOSED_HOOKS
    }

/** Decode a backend's installed-hook wire mask into the typed registry set. */
internal fun installedHooks(statusRaw: String): Set<HookIds.Hook> = parseProtocolStatusBlock(statusRaw)?.hooks?.let(::hooksInMask).orEmpty()

/** Whether any hook that should cover this vector is in [hooks]. */
internal fun NativeCheckSpec.coveredBy(hooks: Set<HookIds.Hook>): Boolean = expectedHooks.any { it in hooks }
