# Which native backend to use

Native hiding covers detection below the Java framework, including libc, direct
syscalls, netlink and `/proc/net`. Install the app and the **full `vpnhide-core.zip`
from the same build**. Core includes the backend payloads and selects a compatible
implementation automatically.

## The choices

- **Built-in:** Core uses a compatible VPN Hide driver already integrated into
  your kernel. Installing Core does not add this driver to a stock kernel.
- **Kernel module (Kmod):** Core selects a compatible bundled `.ko` using kernel
  evidence. See [the kernel-module guide](kmod-install.md).
- **KPM:** a bundled kernel backend requiring a working authenticated KernelPatch
  runtime. See [KPM requirements](kpm-install.md).
- **Zygisk:** a bundled userspace fallback requiring a compatible Zygisk provider.
  Direct syscalls can bypass libc hooks. See [Zygisk requirements](zygisk-install.md).

Java/LSPosed and Native cover different paths. The Apps role needs Java/LSPosed;
Ports is part of Core. See [Detection vectors](detection-vectors.md).

## Keep one native backend

Core keeps native backends mutually exclusive. All alternatives inside the
universal Core ZIP belong together; their presence is not a conflict. Old separate
VPN Hide modules or an independently loaded backend may need migration. Follow
Core's recovery guidance and keep your configuration and working installation
until the replacement is prepared and verified. Do not load kernel payloads by hand.

## Let the Dashboard pick

Core makes the selection; the Dashboard shows selection, application and retained
diagnostic evidence. Missing compatibility or incomplete coverage remains visible.
A backend being present is not proof that protection works. See
[Installed but not active](module-not-active.md).
