# Updating the app and modules

VPN Hide is the app plus the **VPN Hide Core** module and one native backend
module. Keep their versions **in step** — update the core and backend modules when
you update the app, and reboot afterwards. The app and the core must come from
the same release: with a mismatched core the app shows **VPN Hide Core version
mismatch** until both match.

## How to update

- **The app** — install the newer APK from the release over the old one.
- **A module** (core, kernel module, KPM, Zygisk) — install the newer ZIP
  through your root manager's **Modules** screen (KernelSU / Magisk / APatch),
  then **reboot**.

## Upgrading from 1.x

2.0 replaced the per-module activators with the core module. Install
`vpnhide-core.zip`, update your native backend ZIP, then **uninstall** the old
**VPN Hide (Ports)** and **VPN Hide (Built-in kernel)** modules if present — 2.0
has no such modules and their old boot scripts interfere with the core (the
Dashboard shows an error while one is installed). Reboot. Your saved app roles
carry over.

## Version-mismatch warnings

The Dashboard tells you when versions drift apart:

- *"Module version X is older than app version Y"* — update that module (open
  your root manager → Modules).
- *"Module version X is newer than app version Y"* — update the VPN Hide app.

Follow the warning for the specific component. Do not use the size of the version
gap as evidence that a mismatch is harmless.

## It stopped working right after a system update

A system or kernel update can change the kernel so the `.ko` no longer loads
(its GKI interface moved), or reset your LSPosed scope. That's a separate
situation with its own fixes — see
[It stopped working after an update](broke-after-update.md).

## APK updates also need a reboot

After replacing the APK, the app process may be new while the LSPosed code already
loaded in `system_server` is old. Reboot even if you updated only the APK. Then
check the Dashboard for running-version and apply warnings; a reboot cannot fix
an old ZIP that was never updated.

Use components from the [same official release](https://github.com/okhsunrog/vpnhide/releases)
where available. For self-built Built-in integrations, replacing the APK or core
module does not replace kernel code: update your own kernel integration separately.
