# Install the KPM backend

KPM is a kernel backend bundled inside the full Core ZIP. It requires a separate
KernelPatch runtime. Install [the paired APK and Core](first-install.md).

## It needs a KernelPatch runtime

Core must confirm an applicable runtime and authenticate it. A root-manager name
or a directory alone does not establish readiness. Core does not include or flash
a KernelPatch engine. Runtime provisioning is a separate installation operation.

## Steps

1. Confirm that your kernel and runtime are supported and ready.
2. Install the full Core ZIP and matching APK.
3. Supply a SuperKey only if Core requests it and trusted-token access is unavailable.
4. Reboot when required, unlock and review Core's result.

Core owns KPM loading and configuration. Do not extract/load an inner payload or
install a separate VPN Hide KPM ZIP alongside Core.

## SuperKey storage

If authentication needs your APatch/FolkPatch SuperKey, use **Settings → Security**.
The saved copy is root-only; clearing it removes that copy. Never include the key
in a report. See [Security and privacy](security-and-privacy.md#saved-credentials).

## Supported kernels

Support depends on validated kernel layouts and the actual runtime, not only a
version label. Unknown compatibility remains unverified. Core reports applicability
for your device and does not assume that all non-GKI kernels work.

## Why it shows 10 hooks on some kernels

An installed-hook count describes setup, not complete diagnostic coverage.
Different kernels expose different paths. Missing or unverified required checks
remain visible even when some hooks are installed.

## If it's installed but inactive

Check runtime authentication, kernel applicability and Core's pending/error state.
Repeated installation of the same payload does not fix unsupported offsets.
See [Installed but not active](module-not-active.md) or
[collect a report](collect-report.md).

## Where to download and how to install KPatch-Next-Module

KernelPatch provisioning is separate from VPN Hide's universal installer. Use a
supported runtime installation path for your actual root/kernel combination and
retain a recoverable original boot image. A package name alone does not establish
compatibility. Core's KPM payload is not an engine installer; do not apply an old
unverified boot-patching recipe to resolve a Core status error.
