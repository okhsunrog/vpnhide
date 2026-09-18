# Install the KPM backend

KPM is a kernel-level native backend for the kernels the loadable `.ko` **can't**
serve — old or non-GKI kernels (e.g. 4.9, 4.14), vendor kernels with no source,
or kernels that reject unsigned modules. It hides at the kernel level just like
the `.ko`, without injecting libc hooks into the target process. This does not
guarantee invisibility to all anti-tamper or root checks. It's a **single universal
module** — there's no GKI variant to match. See
[Which native backend to use](choosing-native.md) first.

## It needs a KernelPatch runtime

KPM uses inline kernel hooks, which require the KernelPatch runtime to be present
in your kernel. Pick the path that matches your root:

- **APatch / FolkPatch** — KernelPatch is already built in. Just install the
  module ZIP and save your **SuperKey** (below).
- **Magisk / KernelSU / KernelSU-Next** — use **KPatch-Next-Module** and its
  instructions to patch the kernel, or integrate KPatch-Next into a self-built
  kernel. Installing VPN Hide alone does not provide this runtime.

If the runtime isn't present, the Dashboard says *"the kernel is not patched with
a KernelPatch runtime."*

## Steps

1. Make sure a KernelPatch runtime is present (above).
2. Download `vpnhide-kpm.zip` (the Dashboard names it).
3. Install the **whole ZIP** through your root manager's **Modules** screen, as
   an APM/Magisk-compatible module. Do **not** extract or load the inner
   `vpnhide.kpm` file by itself — on its own it has no activator, boot scripts,
   or config delivery, and hiding won't work.
4. On APatch / FolkPatch, save your **SuperKey** in **Settings → Security** (the
   same one you set in APatch/FolkPatch).
5. **Reboot**, then open the **Dashboard** — the Native backend should read
   **Active**.

## Supported kernels

KPM validates your kernel family at load time. Supported families are 4.4, 4.9, 4.14,
4.19, 5.4, 5.10, 5.15, 6.1, 6.6, and 6.12. Other families are refused rather than
guessed. If the Dashboard shows *"kernel not supported … no validated offset
table,"* use the [Zygisk backend](zygisk-install.md) instead.

## Why it shows 10 hooks on some kernels

On kernels older than 5.3 the Native backend installs 10 hooks, not 11, and reads
**OK** — this is a complete install, not a missing hook. The one hook it skips
guards binding a socket to the VPN interface, and on those kernels the kernel
already blocks that itself (it needs a privilege an ordinary app doesn't have),
so VPN Hide leaves it to the kernel. From 5.3 up all 11 are installed. (The hook
count is separate from the number of Diagnostics checks — see
[What the self-test checks](what-the-check-proves.md).)

## If it's installed but inactive

The Dashboard names the cause:

- **Awaiting / needs SuperKey** — APatch/FolkPatch couldn't load it with the
  trusted `su` token. Save your SuperKey in **Settings → Security** and reboot
  (or tap Refresh). Reinstalling the ZIP does not help.
- **Needs a runtime** — no KernelPatch runtime in the kernel; install one (above).
- **Standalone install detected** — the bare `vpnhide.kpm` was loaded but the
  module ZIP isn't installed. Remove the standalone entry, install the full
  `vpnhide-kpm.zip` from the Modules screen, and reboot.
- **Conflict with the kernel module** — the `.ko` and KPM are both installed;
  they hook the same functions and can hard-freeze the device, so KPM stands
  down. Uninstall one of them. **Never run both.**

For a load failure, use **Settings → Debugging → Collect debug log** to attach
the boot-time output to a report — see [Collect a debug report](collect-report.md).

## Runtime installation instructions

Use the [official KPatch-Next-Module instructions](https://github.com/KernelSU-Next/KPatch-Next-Module)
for its supported root managers, including KernelSU-Next, or the
[KPatch-Next project](https://github.com/KernelSU-Next/KPatch-Next) for kernel integration.
VPN Hide's ZIP does not install or patch the KernelPatch runtime for you.
