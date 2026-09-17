# Requirements and compatibility

VPN Hide requires **root**. Enable the layers needed for the ways your target app
detects VPNs; Java + Native is the usual starting point, not an all-or-nothing requirement.

| Component | Requires |
|---|---|
| App | Android 9+, arm64 or armv7, root access granted to VPN Hide |
| Java and Apps roles | LSPosed / LSPosed-Next / Vector, with VPN Hide enabled for System Framework |
| Native — kmod | Compatible arm64 GKI kernel with the required probe support; use the Dashboard's recommendation |
| Native — KPM | Supported arm64 kernel family and KernelPatch runtime, such as APatch or KPatch-Next |
| Native — Zygisk | A working Zygisk implementation, such as Magisk Zygisk or ZygiskNext/NeoZygisk |
| Ports | Ports module installed through a supported root manager |

Kernel backends are arm64-only. On a device without a compatible kernel backend,
[Zygisk](zygisk-install.md) is a fallback with reduced coverage. KernelSU does not
by itself supply the Zygisk implementation needed by that backend.

## What to install

Start with [First install](first-install.md), then [choose one native backend](choosing-native.md).
Kernel hooks avoid injecting libc hooks into the target process, but do not promise
invisibility to every anti-tamper or root check. VPN Hide does not hide root or
provide Play Integrity attestation.

Built-in is an advanced **self-built kernel integration**, not a supported ready-made
kernel download. It is described separately in [Choosing Native](choosing-native.md).
