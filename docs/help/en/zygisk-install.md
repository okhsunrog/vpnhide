# Install the Zygisk module

Zygisk is the userspace native backend: it hooks libc **inside each target
app's process**. Use it when no kernel backend fits your device. It's weaker
than the kernel backends — see [Which native backend to use](choosing-native.md)
before choosing it.

## What to expect

- **Raw `svc` syscalls bypass it.** An app that issues syscalls directly, skipping
  libc, isn't filtered. Kernel backends (kmod / KPM) don't have this gap.
- **Banking and payment apps may detect it.** The hooks live in the app's own
  process, which some anti-tamper SDKs notice. For those apps, leave **Native
  off** and rely on the Java layer — turning Native on can make them *more*
  suspicious, not less. The Dashboard shows the same warning.

If your kernel supports kmod or KPM, prefer one of those; the Dashboard will
tell you.

## Requirements

- A **Zygisk implementation**: stock Magisk Zygisk, or ZygiskNext / NeoZygisk on
  Magisk or KernelSU.
- **LSPosed / Vector** is recommended alongside it for Java/Apps coverage.
  Zygisk alone does not provide those roles.

## Steps

1. Download `vpnhide-zygisk.zip` (the Dashboard names it).
2. In your root manager, open **Modules → Install from storage** and pick the ZIP.
3. **Reboot.**
4. Pick target apps in the VPN Hide app.
5. Open the **Dashboard**; the Native backend should read **Active**.

## Applying changes

Zygisk hooks are installed when the app process starts, so a config change takes
effect the next time the app is launched fresh. **Force-stop** a target app and
reopen it to apply — see [Set up hiding](configure-hiding.md). A reboot is needed
only to install, update, disable or remove the module itself; changing targets
or hooks never needs one.

The module takes its targets from `targets.txt` in its module folder. Save
rewrites that file from the saved configuration (so do every boot and a fresh
start of VPN Hide), and the module reads it anew in every app process it is
injected into. Apps are listed there by UID, so after reinstalling a target app
tap Save again before force-stopping it; see
[When changes take effect](saving-applying.md).
