# Pause, reset and uninstall

## Pause hiding for an app

Clear its roles in **Hiding**, Save successfully, then force-stop and reopen it.
This also clears in-process Zygisk hooks on the next launch. Merely disabling a
module in the root manager does not remove already loaded hooks; reboot when
changing the installation/enabled state. Save a [backup](config-backup.md) first
if you want to restore your selection later.

## Remove the installation and leftover files

1. Export your configuration if you may need it later.
2. Clear roles and Save while Core is still available. Disable VPN Hide in LSPosed.
3. Open **Settings → Remove leftover files** and inspect the blockers before
   removing Core. Loaded kernel or process hooks may require a user-chosen reboot
   or a kernel without VPN Hide. Removing a file does not unload them.
4. Run cleanup only when Core reports that it is safe. If this beta cannot reach
   that state, keep the configuration and Core and collect a report; do not wipe
   the journal or bypass an unknown-effect hold.
5. After completed cleanup, remove **VPN Hide Core** through the root manager and
   uninstall the APK. Follow the manager's reboot instructions.

The unified installation has no separate backend ZIP to uninstall. Legacy modules
from an older installation require the explicit migration/recovery path. Cleanup
removes persistent configuration irreversibly; uninstalling the APK alone does
not perform it.

## Self-built Built-in kernels

This advanced integration is not a normal downloadable kernel option, and it has
no module to remove: the code is compiled into the running kernel. Full cleanup
requires **booting a kernel without VPN Hide**, as well as removing the other
modules and disabling LSPosed. Another reboot into the same
Built-in kernel will not resolve that blocker. Plan how to restore your kernel
before dismantling a self-built integration.
