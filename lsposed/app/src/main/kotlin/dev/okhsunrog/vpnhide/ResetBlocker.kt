package dev.okhsunrog.vpnhide

// Full reset: core (`root_effect` `FullReset`) deletes the service files VPN Hide
// and its modules wrote outside their Magisk module dirs (/data/system,
// /data/adb/vpnhide*), which survive an APK uninstall. Module dirs, runtime state
// and the app's own data are out of scope. See core-daemon `effects.rs`.

// A reason the full reset must wait — something VPN Hide left running is still
// installed/active, so deleting its state now is unsafe (a live .ko keeps
// filtering; an active hook re-reads the config). The user must clear these and
// reboot first.
internal enum class ResetBlocker {
    KmodInstalled,
    BuiltinInstalled,
    KpmInstalled,
    ZygiskInstalled,
    PortsInstalled,
    KernelStillHooked,
    LsposedActive,
}

/**
 * Preconditions for a safe full reset, reusing the dashboard's tested module /
 * hook detection. Empty list == ready. A backend whose module dir is still
 * present (even disabled) blocks, as does a still-loaded .ko
 * ([kernelCtlPresent], from `/proc/vpnhide_ctl`) or an LSPosed hook active this
 * boot.
 */
internal fun resetBlockers(
    kmod: ModuleState,
    kpm: ModuleState,
    zygisk: ModuleState,
    ports: ModuleState,
    lsposed: LsposedState,
    kernelCtlPresent: Boolean,
    builtin: ModuleState = ModuleState.NotInstalled,
): List<ResetBlocker> =
    buildList {
        if (kmod is ModuleState.Installed) add(ResetBlocker.KmodInstalled)
        if (builtin is ModuleState.Installed) add(ResetBlocker.BuiltinInstalled)
        if (kpm is ModuleState.Installed) add(ResetBlocker.KpmInstalled)
        if (zygisk is ModuleState.Installed) add(ResetBlocker.ZygiskInstalled)
        if (ports is ModuleState.Installed) add(ResetBlocker.PortsInstalled)
        if (kernelCtlPresent) add(ResetBlocker.KernelStillHooked)
        if (lsposed is LsposedState.Active) add(ResetBlocker.LsposedActive)
    }
