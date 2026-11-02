package dev.okhsunrog.vpnhide

internal fun sampleNativeStatus(
    selected: NativeBackendId = NativeBackendId.Builtin,
    observed: NativeBackendId = selected,
    current: Boolean = true,
    boot: NativeBootStatus? = null,
    installedHooks: Long = KERNEL_HOOKS.toHookMask(),
): NativeStatusSnapshot {
    val implementation = NativeImplementationInfo(selected, "builtin", null, null, null)
    return NativeStatusSnapshot(
        current,
        7,
        3,
        1,
        0,
        NativeStatusInfo(
            boot = boot,
            selected = implementation,
            applied = selected,
            observed = observed,
            displayed = implementation,
            observation = "current",
            runtime = NativeRuntimeInfo(observed, 0, installedHooks, 0),
            observationError = null,
            attempts = emptyList(),
            delivery = NativeDeliveryInfo(selected, 1, 0, "confirmed", "a".repeat(64)),
            requiredHooks = installedHooks,
            installedHooks = installedHooks,
            missingHooks = 0,
            setup = "setup_ready",
            filesystem = "disabled",
            processCoverageComplete = false,
            legacyInstalled = emptyList(),
        ),
    )
}
