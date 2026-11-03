# Updating the app and modules

VPN Hide uses the APK and one full **VPN Hide Core** ZIP. Core contains the
native payloads and selects the applicable backend. Use compatible artifacts from
the [same official release](https://github.com/okhsunrog/vpnhide/releases).
This beta's Zygisk publication lifecycle is still under review; package layout
alone does not establish that the backend is ready.

## How to update

- Install the newer APK over the old one.
- For a Core update, install the **full Core ZIP** through your root manager's
  Modules screen, then reboot to activate the staged replacement.
- Follow the Dashboard's compatibility and running-version information.

A compatible client/UI APK update does not itself require reinstalling Core.
Do not download a separate kmod, KPM or Zygisk ZIP or choose a GKI payload manually.

## Upgrading from 1.x

Prepare and verify the replacement before removing old modules. Preserve your
saved configuration and follow the migration instructions for the full Core
package. Old Ports/Built-in modules and standalone backend boot scripts belong
to the old layout; a prepared replacement must be ready before they are retired.
The common automatic maintenance flow is still being implemented in this beta.

## Version-mismatch warnings

Follow the warning for the specific running component. Check the APK/Core
protocol compatibility and actual loaded generation, rather than assuming that
similar version numbers are sufficient. A Core RPC or journal version change
does not by itself require rebuilding a user-integrated Built-in kernel.

## It stopped working right after a system update

A kernel update can change module compatibility or LSPosed scope. Core derives
kernel candidates from its verified catalog. Keep the reported failure and see
[It stopped working after an update](broke-after-update.md); do not try arbitrary
backend archives.

## APK updates also need a reboot

A reboot is needed when changed LSPosed code must replace code already loaded
in `system_server`. Changes confined to a compatible client/UI APK have a
different lifecycle. Check the reported running version and required action.
A reboot cannot replace an old Core package that was never updated.

A self-built Built-in integration is kernel code: replacing the APK or Core ZIP
does not replace that integration.

## Updating through the app

With a current compatible Core connection and an authenticated target embedded
in the APK, the Dashboard offers **Choose Core update ZIP**. Select the matching
full Core ZIP. Core checks the signature, exact package and staged result before
reporting that a restart is needed. You decide when to reboot; the app never
reboots the phone automatically. A prepared update is not yet the running Core.
If confirmation is lost, check the same update's status instead of starting
another installation. Unconfigured developer builds do not offer this action.

Without Core or with incompatible APK/Core RPC versions, use the root manager's
Modules screen as described above; the app has no su installer. The unified
missing-Core bootstrap/handoff is still being implemented.
