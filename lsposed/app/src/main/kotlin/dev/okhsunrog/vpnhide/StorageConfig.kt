package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import uniffi.vpnhide_core_ffi.CoreRoleSets
import uniffi.vpnhide_core_ffi.encodeCanonicalInterchange
import uniffi.vpnhide_core_ffi.previewRoleConfig
import uniffi.vpnhide_core_ffi.previewSelfTarget

internal fun nativeHookFamilyFor(backend: NativeBackendId?): NativeHookFamily =
    when (backend) {
        NativeBackendId.Zygisk -> NativeHookFamily.Zygisk

        NativeBackendId.Kmod -> NativeHookFamily.Kmod

        // The in-tree backend owns the same kernel hooks as the .ko (including the
        // optional filesystem hook), so it shares the kmod config hook family.
        NativeBackendId.Builtin -> NativeHookFamily.Kmod

        NativeBackendId.Kpm -> NativeHookFamily.Kpm

        null -> NativeHookFamily.Kernel
    }

internal fun buildCanonicalConfig(
    debug: Boolean,
    // null = inherit the user's toggle intent from [existing] (or fall back to
    // [debug] on a first-write). Rebuild paths (Save / auto-hide / manual-hidden)
    // must NOT reset debugSwitch to debug — the two diverge during a capture, and
    // defaulting to debug would clobber the persisted intent.
    debugSwitch: Boolean? = null,
    javaPkgs: Collection<String>,
    nativePkgs: Collection<String>,
    hiddenPkgs: Collection<String>,
    observerPkgs: Collection<String>,
    portsPkgs: Collection<String>,
    existing: CanonicalConfig? = null,
): CanonicalConfig =
    previewRoleConfig(
        debug,
        debugSwitch,
        CoreRoleSets(javaPkgs.toList(), nativePkgs.toList(), hiddenPkgs.toList(), observerPkgs.toList(), portsPkgs.toList()),
        existing?.toCore(),
    ).toCanonical()

internal fun buildCanonicalConfigFromTargetsSnapshot(
    snapshot: TargetsSnapshot,
    debug: Boolean = snapshot.canonicalConfig?.debug ?: false,
    debugSwitch: Boolean = snapshot.canonicalConfig?.debugSwitch ?: debug,
): CanonicalConfig =
    buildCanonicalConfig(
        debug = debug,
        debugSwitch = debugSwitch,
        javaPkgs = snapshot.lsposedTargets,
        nativePkgs = snapshot.nativeTargets,
        hiddenPkgs = snapshot.hiddenPkgs,
        observerPkgs = snapshot.observerNames,
        portsPkgs = snapshot.portsObservers,
        existing = snapshot.canonicalConfig,
    )

internal fun canonicalConfigWithSelfTarget(
    config: CanonicalConfig,
    selfPkg: String,
): CanonicalConfig = previewSelfTarget(config.toCore(), selfPkg).toCanonical()

internal fun canonicalConfigJson(config: CanonicalConfig): String = encodeCanonicalInterchange(config.toCore())
