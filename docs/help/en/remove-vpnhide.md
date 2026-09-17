# Pause, reset and uninstall

## Pause hiding for an app

Clear its roles in **Hiding**, Save successfully, then force-stop and reopen it.
This also clears in-process Zygisk hooks on the next launch. Merely disabling a
module in the root manager does not remove already loaded hooks; reboot when
changing the installation/enabled state. Save a [backup](config-backup.md) first
if you want to restore your selection later.

## Remove the installation and leftover files

1. Export your config if you may need it later.
2. Remove VPN Hide's native module and Ports module in the root manager.
3. Disable VPN Hide in LSPosed, then **reboot**.
4. Open **Settings → Remove leftover files**. Resolve any listed blockers first.
5. Run cleanup, then use the offered uninstall action to remove the APK.

Cleanup deletes configuration and other persistent VPN Hide files outside module
folders; it cannot be undone. Uninstalling just the APK does not perform this cleanup.

## Self-built Built-in kernels

This advanced integration is not a normal downloadable kernel option. Removing
its companion does not remove code compiled into the running kernel. Full cleanup
requires removing the companion **and booting a kernel without VPN Hide**, as well
as removing other modules and disabling LSPosed. Another reboot into the same
Built-in kernel will not resolve that blocker. Plan how to restore your kernel
before dismantling a self-built integration.
