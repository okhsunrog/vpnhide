# First install

VPN Hide is a few pieces working together: this app (the picker), one **native**
backend, the **Java** layer via LSPosed, and an optional **Ports** module. Check
[Requirements and compatibility](requirements.md) first.

The steps below describe the recommended Java + Native setup. You may omit a layer,
but its detection paths remain uncovered; Ports is optional. See [Requirements](requirements.md).

## Steps

1. **Install the app** — the picker APK from the project's releases. It's the
   dashboard you're reading this in.
2. **Grant it root.** VPN Hide needs root to manage hiding. Open the app and
   grant it in your root manager's prompt (Magisk / KernelSU / APatch). If you
   miss the prompt, the app shows a **Root access required** screen — grant it,
   then tap **Check again**.
3. **Install one native backend.** The Dashboard recommends the best one for your
   device. Install its module (the ZIP) through your root manager's **Modules**
   screen — Magisk, KernelSU, APatch or FolkPatch. Install only one native
   backend; two active at once can conflict.
4. **Turn on the Java layer.** In LSPosed / LSPosed-Next / Vector, enable
   **VPN Hide** and add **System Framework** to its scope. Without that scope the
   Java hooks don't attach.
5. **(Optional) Install Ports** if you need localhost port hiding.
6. **Reboot** so the native and Java modules load.

## Check it worked

Open the **Dashboard**. Each installed module should read **Active**, and the
hero shows **VPN hidden** once a VPN is up and its checks pass. If a module says
"reboot to activate" or "not active", follow the on-screen hint. Missing scope,
a not-yet-rebooted module, or two native backends at once are the usual causes.

Then move on to [Set up hiding](configure-hiding.md).

## Official downloads

Get the APK and module ZIPs from the [project releases](https://github.com/okhsunrog/vpnhide/releases). Choose the native ZIP recommended for your device; a kernel ZIP is not interchangeable between all phones.
