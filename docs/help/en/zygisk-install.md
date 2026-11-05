# Install the Zygisk module

Zygisk provides native hiding through libc hooks inside target processes. Both
ABI payloads are included once in the full Core ZIP. Use
[the Core installation](first-install.md); no separate VPN Hide Zygisk ZIP is needed.

## What to expect

**Prefer a compatible kernel backend.** Zygisk hooks run inside app processes;
banking, anti-fraud and other protected apps may detect the provider or injected
code and refuse to work. Zygisk is a fallback with reduced coverage, not the
recommended starting point. A kernel backend does not guarantee invisibility or
hide an installed Zygisk provider or root. Direct syscalls can bypass libc hooks.

Java/LSPosed covers a separate layer; consult Core's recorded scope and missing evidence.

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

In Settings → Native backend (advanced), select Zygisk and read/confirm its injection warning before saving. A compatible provider must already be available. Core prepares its managed runtime and reports when a device restart is required. Staging, provider discovery, current-boot authorization, actual code consumption and hook health are separate; a pending or unverified result is not protection. Auto never silently enters Zygisk. Existing old separate installations can require an isolated migration path; keep them when Core refuses an unverified transition.

When upgrading an old Core that already uses Zygisk, the installer can retain that
choice only from exact confirmed Core policy and runtime records. It stages the
matching managed payload without changing the running policy; a real reboot and
new code verification are required. An unused old payload is not enabled. If Core
cannot prove continuity, keep the old installation and follow its refusal message.
For later updates of an already managed Zygisk runtime, use Core’s paired updater;
a different manual Core-only ZIP is refused to preserve the current runtime.
