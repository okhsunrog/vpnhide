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

/** The hooks owned by [backend] (its generated mask), or an empty set for an unknown id. */
internal fun ownedHooks(backend: HookIds.Backend): Set<HookIds.Hook> =
    when (backend) {
        HookIds.Backend.KMOD -> KERNEL_HOOKS + KMOD_HOOKS
        HookIds.Backend.BUILTIN -> KERNEL_HOOKS + BUILTIN_HOOKS
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

        NativeBackendId.Builtin -> {
            KERNEL_HOOKS + installedOptionalHooks.intersect(BUILTIN_HOOKS)
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
        // Builtin is a kernel backend like the .ko/KPM: same KERNEL_HOOKS, same
        // by-name resolve-at-load, so it reports PARTIAL_HOOKS the same way.
        NativeBackendId.Kmod, NativeBackendId.Builtin, NativeBackendId.Kpm -> {
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
        HookIds.Backend.BUILTIN -> KERNEL_HOOKS + installed.intersect(BUILTIN_HOOKS)
        HookIds.Backend.KPM -> KERNEL_HOOKS + installed.intersect(KPM_HOOKS)
        else -> ownedHooks(backend)
    }

/** Whether any hook that should cover this vector is in [hooks]. */
internal fun NativeCheckSpec.coveredBy(hooks: Set<HookIds.Hook>): Boolean = expectedHooks.any { it in hooks }
