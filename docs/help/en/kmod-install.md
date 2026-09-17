# Install the kernel module

The kernel module (the `.ko`, shown as **Kmod** on the Dashboard) is the default
native backend on GKI kernels. It hooks in kernel space, so the target app's
libc hooks are not injected into the target process. This avoids that particular
in-process detection surface; it does not hide root or guarantee every app will accept it. First make
sure it's the right backend for your device — see
[Which native backend to use](choosing-native.md).

## Match the GKI variant

The `.ko` is built per **GKI generation**. You install the ZIP that matches your
kernel's GKI Kernel Module Interface (KMI) tag — e.g. `android14-6.1`. That tag
is part of the kernel version string; it names the kernel's binary interface to
modules, **not** your Android OS release, so "android14" in the tag doesn't mean
you're on Android 14.

The **Dashboard** reads your kernel and tells you the exact file to download
(for example, `vpnhide-kmod-android14-6.1.zip`). If it can't tell your GKI branch
apart from the version string, it names one to try first and a fallback to use
if that one doesn't load.

## Steps

1. Download the ZIP the Dashboard names.
2. In your root manager (Magisk / KernelSU / APatch), open **Modules → Install
   from storage** and pick the ZIP.
3. **Reboot** — the module loads at boot.
4. Open the **Dashboard**; the Native backend should read **Active**.

Pick target apps in the VPN Hide app; you don't edit any files by hand.

## Updating or removing it

The loaded `.ko` stays resident until you reboot. To update or remove it, do it
through your root manager's **Modules** screen (update the ZIP or disable the
module) and **reboot** to apply. A plain `rmmod` is not supported.

## If it won't load

The Dashboard tells you which case you hit:

- **Installed, wrong GKI variant** — you flashed the ZIP for another generation.
  Install the one the Dashboard names.
- **Installed, did not load — try the other GKI variant** — your GKI branch was
  ambiguous; install the fallback variant the Dashboard suggested.
- **Installed, kernel not supported** / **kernel has no kprobes** — this kernel
  can't load the `.ko`. Use the [KPM backend](kpm-install.md) if your kernel
  family is supported, otherwise the [Zygisk module](zygisk-install.md).
- **Installed, kernel rejects unsigned modules** — module signing is enforced;
  the `.ko` can't be inserted. Use KPM (loaded via KernelPatch, not the module
  loader) or Zygisk.

If it loaded but the Dashboard shows only some hooks installed, a few detection
surfaces on this specific kernel build aren't covered by this backend;
reinstalling won't change that. See
[A module is installed but not active](module-not-active.md).
