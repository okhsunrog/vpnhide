# Install the Zygisk module

Zygisk provides native hiding through libc hooks inside target processes. Both
ABI payloads are included once in the full Core ZIP. Use
[the Core installation](first-install.md); no separate VPN Hide Zygisk ZIP is needed.

## What to expect

Direct syscalls can bypass these hooks, and apps may detect modifications inside
their own process. Use Core's reported coverage when deciding which roles to
configure. Java/LSPosed provides a separate layer.

## Requirements

A compatible Zygisk provider must already be available. Provider discovery,
authorization and hooks applied in an app are separate facts. Availability of
LSPosed alone does not prove that the VPN Hide payload is ready.

## Steps

1. Install the paired APK and universal Core ZIP.
2. Keep the required provider and Java-layer scope configured.
3. Reboot when installation or Core's transition status requires it, then unlock.
4. Select target roles and tap **Save**.

A restart-pending or unverified backend is not active protection. Follow the
Dashboard's action and retain diagnostic errors when asking for support.

## Applying changes

Already-running apps can retain an older hook policy. Force-stop and reopen a
target after changing its configuration. Provider discovery changes can need a
reboot; an app restart alone does not guarantee discovery. Core owns delivery and
current-boot authorization; you do not edit target files. A fresh diagnostic
worker does not prove that every old game process updated.
