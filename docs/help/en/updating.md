# Updating the app and modules

VPN Hide is the app plus one or more modules (a native backend, and maybe Ports).
Keep their versions **in step** — update the modules when you update the app, and
reboot afterwards.

## How to update

- **The app** — install the newer APK from the release over the old one.
- **A module** (kernel module, KPM, Zygisk, Ports) — install the newer ZIP
  through your root manager's **Modules** screen (KernelSU / Magisk / APatch),
  then **reboot**.

## Version-mismatch warnings

The Dashboard tells you when versions drift apart:

- *"Module version X is older than app version Y"* — update that module (open
  your root manager → Modules).
- *"Module version X is newer than app version Y"* — update the VPN Hide app.

They don't have to match to the digit, but a large gap means one half is stale;
bring them together.

## It stopped working right after a system update

A system or kernel update can change the kernel so the `.ko` no longer loads
(its GKI interface moved), or reset your LSPosed scope. That's a separate
situation with its own fixes — see
[It stopped working after an update](broke-after-update.md).
