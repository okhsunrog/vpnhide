package dev.okhsunrog.vpnhide

// Full reset: delete the service files VPN Hide and its modules write OUTSIDE
// their Magisk module directories. These are root-owned and live under
// /data/system and /data/adb/vpnhide*, so they survive an APK uninstall and are
// not removed by any module's uninstall.sh — this is the "leftover files after
// uninstall" users complained about.
//
// Deliberately out of scope (each would only add risk):
//  - /data/adb/modules/vpnhide_* — manager-owned; remove via Magisk/KSU/APatch,
//    not by rm (would desync the module DB).
//  - runtime state (iptables chains, /proc/vpnhide_ctl) — cleared by a reboot
//    once the modules are gone.
//  - the app's own /data/data (SharedPreferences, zygisk heartbeat) — wiped
//    when the APK is uninstalled.

// Plain files under /data/system (rm -f).
internal val FULL_RESET_FILES =
    listOf(
        CANONICAL_CONFIG_FILE,
        "/data/system/vpnhide_uids.txt",
        SS_HIDDEN_PKGS_FILE,
        SS_OBSERVER_UIDS_FILE,
        LSPOSED_STATE_FILE,
        LEGACY_HOOK_STATUS_FILE,
    )

// Data directories under /data/adb (rm -rf — covers targets.txt, load_status,
// superkey, etc. inside). Each is an explicit, vpnhide-prefixed path — never a
// glob and never a module install dir.
internal val FULL_RESET_DIRS =
    listOf(
        "/data/adb/vpnhide",
        "/data/adb/vpnhide_kmod",
        "/data/adb/vpnhide_kpm",
        "/data/adb/vpnhide_zygisk",
        "/data/adb/vpnhide_lsposed",
        "/data/adb/vpnhide_ports",
    )

/** Single su command that removes every leftover service file/dir. Idempotent:
 *  paths a module's uninstall.sh already removed are simply no-ops. */
internal fun buildFullResetCommand(): String =
    (
        listOf("rm -f " + FULL_RESET_FILES.joinToString(" ")) +
            FULL_RESET_DIRS.map { "rm -rf $it" }
    ).joinToString(" ; ")

// A reason the full reset must wait — something VPN Hide left running is still
// installed/active, so deleting its state now is unsafe (a live .ko keeps
// filtering; an active hook re-reads the config). The user must clear these and
// reboot first.
internal enum class ResetBlocker {
    KmodInstalled,
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
): List<ResetBlocker> =
    buildList {
        if (kmod is ModuleState.Installed) add(ResetBlocker.KmodInstalled)
        if (kpm is ModuleState.Installed) add(ResetBlocker.KpmInstalled)
        if (zygisk is ModuleState.Installed) add(ResetBlocker.ZygiskInstalled)
        if (ports is ModuleState.Installed) add(ResetBlocker.PortsInstalled)
        if (kernelCtlPresent) add(ResetBlocker.KernelStillHooked)
        if (lsposed is LsposedState.Active) add(ResetBlocker.LsposedActive)
    }
