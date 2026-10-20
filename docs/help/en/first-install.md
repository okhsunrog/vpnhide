# First install

VPN Hide is a few pieces working together: this app (the picker), the **VPN Hide
Core** module that does all root work, one **native** backend, and the **Java**
layer via LSPosed. Localhost port hiding (**Ports**) is part of the core module. Check
[Requirements and compatibility](requirements.md) first.

The steps below describe the recommended Java + Native setup. You may omit a layer,
but its detection paths remain uncovered; Ports is optional. See [Requirements](requirements.md).

## Steps

1. **Install the app** — the picker APK from the project's releases. It's the
   dashboard you're reading this in.
2. **Install the core module.** Install `vpnhide-core.zip` from the **same
   release** as the app through your root manager's **Modules** screen (Magisk /
   KernelSU / APatch) and keep it enabled. The app itself never asks for root:
   it talks to the core module. Until the core runs (after the reboot in step 6)
   the app shows **VPN Hide Core is not running**; a core from another release
   shows **VPN Hide Core version mismatch**. Upgrading from 1.x? Uninstall the old
   **VPN Hide (Ports)** and **VPN Hide (Built-in kernel)** modules if you have them.
3. **Install one native backend.** The Dashboard recommends the best one for your
   device. Install its module (the ZIP) through your root manager's **Modules**
   screen — Magisk, KernelSU, APatch or FolkPatch. Install only one native
   backend: with more than one, core loads none of them.
   Instructions: [kmod](kmod-install.md) · [KPM and KernelPatch setup](kpm-install.md) ·
   [Zygisk](zygisk-install.md). Unsure? See [choosing a backend](choosing-native.md),
   including kernels with VPN Hide built in.
4. **Turn on the Java layer.** In LSPosed / LSPosed-Next / Vector, enable
   **VPN Hide** and add **System Framework** to its scope. Without that scope the
   Java hooks don't attach.
5. **(Optional) [Ports](ports.md)** — nothing to install; give apps the Ports role later.
6. **Reboot** so the core starts, loads the native backend, and the Java layer loads.

## Check it worked

Open the **Dashboard**. Each installed module should read **Active**, and the
hero shows **VPN hidden** once a VPN is up and its checks pass. If a module says
"reboot to activate" or "not active", follow the on-screen hint. Missing scope,
a not-yet-rebooted module, a missing core module, or two native backends at once
are the usual causes.

Then move on to [Set up hiding](configure-hiding.md).

## Official downloads

Get the APK, `vpnhide-core.zip` and the backend ZIPs from the [project releases](https://github.com/okhsunrog/vpnhide/releases). Choose the native ZIP recommended for your device; a kernel ZIP is not interchangeable between all phones.
