# Turning it off, resetting and uninstalling

## Pause hiding without removing anything

To stop hiding for one app, remove its roles in the picker and Save. To stop
everything temporarily, disable the modules in your root manager (and the
LSPosed module) — you can re-enable them later.

## Remove VPN Hide completely

Order matters, because the app needs to confirm nothing is still active before it
can clean up:

1. In your root manager, **remove all VPN Hide modules** — the native backend
   (kernel module / KPM / Zygisk) and Ports.
2. **Disable the VPN Hide module in LSPosed.**
3. **Reboot.**

## Remove leftover files

VPN Hide and its modules also write config, state, statistics and target files
**outside** their module folders. To delete those, use **Settings → Remove
leftover files**. It permanently deletes them and **can't be undone**.

The screen won't run until the steps above are done — if a module is still
installed or a hook is still loaded, it lists exactly what's blocking (kernel
module, KPM, Zygisk, Ports, a kernel hook awaiting reboot, or the LSPosed hook)
so you can clear it and reboot first. Once it finishes, it offers to **Uninstall
VPN Hide** (the app itself).

If you only reflash or move devices, you probably want to keep the config —
[export it first](config-backup.md).
