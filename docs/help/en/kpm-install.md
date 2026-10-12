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

- **APatch / FolkPatch** — KernelPatch is already built in. Install the
  module ZIP; a saved **SuperKey** is needed only if trusted-token activation
  is unavailable (see [SuperKey storage](#superkey-storage)).
- **Magisk / KernelSU / KernelSU-Next** — use [**KPatch-Next-Module**](https://github.com/KernelSU-Next/KPatch-Next-Module/releases/latest) and the
  setup steps below, or integrate KPatch-Next into a self-built
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
4. On APatch / FolkPatch, save your **SuperKey** only if requested; see
   [SuperKey storage](#superkey-storage).
5. **Reboot**, then open the **Dashboard** — the Native backend should read
   **Active**.

## SuperKey storage

Save a SuperKey only if APatch/FolkPatch cannot activate KPM with its trusted
`su` token and the Dashboard asks for a key. In **Settings → Security**, use the
key you set in APatch/FolkPatch. Saving stores it in a root-only file that survives
app and device restarts; it is not limited to this session. Clear removes the
saved copy. A later activation may need the key again if trusted-token access
is unavailable. See [Security and privacy](security-and-privacy.md#saved-credentials).

## Supported kernels

KPM validates your kernel family at load time. Supported families are 4.4, 4.9, 4.14,
4.19, 5.4, 5.10, 5.15, 6.1, 6.6, and 6.12. Other families are refused rather than
guessed. If the Dashboard shows *"kernel not supported … no validated offset
table,"* use the [Zygisk backend](zygisk-install.md) instead.

## Why it shows 10 hooks on some kernels

With optional filesystem hiding off, a complete KPM installation reports
**OK · hooks: 10** on kernels older than 5.3 and **OK · hooks: 11** from 5.3 up.
The older kernels already require a privilege ordinary apps do not have to bind
a socket to an interface, so KPM does not install its socket-bind hook there.

The counter counts installed logical hook IDs/groups, not physical interception
points. If **Hide VPN filesystem paths** is enabled and its group installs after
a reboot, it adds one: the counts become **11** and **12**, respectively. One
group can use several interception points. The count also differs from the
number of Diagnostics checks — see
[What the self-test checks](what-the-check-proves.md).

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

## Where to download and how to install KPatch-Next-Module

For Magisk / KernelSU / KernelSU-Next:

1. Download [KPatch-Next-Module.zip](https://github.com/KernelSU-Next/KPatch-Next-Module/releases/latest/download/KPatch-Next-Module.zip)
   from the [official releases](https://github.com/KernelSU-Next/KPatch-Next-Module/releases/latest).
   This provides the KPM runtime, not `vpnhide-kpm.zip`; you need both modules.
2. Install the ZIP through your root manager's **Modules** screen, then open
   the KPatch-Next module's WebUI. If the manager cannot open it, the module's
   action offers [KsuWebUIStandalone](https://github.com/KOWX712/KsuWebUIStandalone/releases).
3. Open **Patch** in the WebUI and patch the boot image. The modified image
   must be flashed and the device rebooted before the runtime is available
   in the running kernel. Installing the ZIP alone is not enough.
   Before flashing, keep a working boot image and a way to restore it.
4. After rebooting, check for **Working** in the WebUI. Then install
   `vpnhide-kpm.zip` using the steps above; if already installed, open VPN Hide
   and refresh its status.

APatch / FolkPatch normally need no separate KPatch-Next-Module: their patched
kernel includes the runtime. If VPN Hide still reports a missing runtime,
check its status in your root manager and collect a
[debug report](collect-report.md). A screenshot alone cannot distinguish a
missing runtime from a failure to detect it.

If patching is unavailable or unsuitable for your device, use
[Zygisk](zygisk-install.md) instead of KPM. For a self-built kernel, see the
[KPatch-Next project](https://github.com/KernelSU-Next/KPatch-Next).
