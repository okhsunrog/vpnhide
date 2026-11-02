# Install the kernel module

The kernel-module backend works in kernel space. Its payloads are included in the
full Core ZIP; use the [single Core installation](first-install.md).

## Match the GKI variant

Core derives compatible candidates from its verified payload catalog and kernel
evidence. A KMI name such as `android14-6.1` describes a kernel module interface,
not the phone's Android release. No manual GKI choice or trial ZIP is needed.
An ambiguous or unsupported kernel is not treated as compatible.

## Steps

1. Install the APK and full `vpnhide-core.zip` from the same release/build.
2. Reboot and unlock the device.
3. Review the Dashboard, choose app roles and tap **Save**.

Core loads and configures the selected backend. Follow its explicit recovery or
restart guidance when selection cannot complete.

## Updating or removing it

Update the paired APK/Core distribution. A loaded `.ko` stays resident until
reboot; ordinary `rmmod` is unsupported. Removing Core affects the whole service.
Keep the working installation until its replacement is verified.

## If it won't load

Module-signature enforcement, missing kernel facilities, ambiguous compatibility
or incomplete hooks can prevent this backend from providing protection. Core
reports the reason and evaluates eligible alternatives without truncating policy.
A failed or unknown load is not permission to try another payload yourself.
See [KPM](kpm-install.md), [Zygisk](zygisk-install.md) and
[collecting a report](collect-report.md).
