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
- **LSPosed / Vector** enabled for the Java layer (the same companion every
  backend needs).

## Steps

1. Download `vpnhide-zygisk.zip` (the Dashboard names it).
2. In your root manager, open **Modules → Install from storage** and pick the ZIP.
3. **Reboot.**
4. Pick target apps in the VPN Hide app.
5. Open the **Dashboard**; the Native backend should read **Active**.

## Applying changes

Zygisk hooks are installed when the app process starts, so a config change takes
effect the next time the app is launched fresh. **Force-stop** a target app (or
reboot) and reopen it to apply — see [Set up hiding](configure-hiding.md).
