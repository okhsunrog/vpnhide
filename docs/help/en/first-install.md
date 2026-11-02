# First install

Install the VPN Hide APK and **one full Core ZIP from the same release/build**.
Core contains the native alternatives and Ports. Java/Apps coverage also needs
LSPosed or a compatible implementation. Check
[Requirements and compatibility](requirements.md).

## Steps

1. **Install the app** from the paired distribution.
2. **Install `vpnhide-core.zip`** through your root manager's Modules screen and
   keep Core enabled. The APK communicates with Core and does not request root.
3. **Let Core select Native.** The universal ZIP includes the payloads; see
   [backend selection](choosing-native.md) and [KPM requirements](kpm-install.md).
4. **Enable the Java layer** in LSPosed / Vector and add **System Framework** to
   VPN Hide's scope.
5. **Ports is optional:** assign its role to apps later; it is included in Core.
6. **Reboot and unlock** to complete installation.

For an upgrade, retain configuration and the working installation until the
replacement is prepared and verified. Follow Core's migration guidance for old
separate modules.

## Check it worked

Open the Dashboard and review Core readiness and retained diagnostics. These do
not depend on an external VPN being enabled or this APK being routed through it.
Pending, failed or incomplete evidence is not a successful protection result.
Then follow [Your first app](first-setup.md), choose roles and tap **Save**.

## Official downloads

Use the paired APK and full `vpnhide-core.zip` from
[project releases](https://github.com/okhsunrog/vpnhide/releases) or the supplied
matching build. The daemon-only base archive is an internal build input, not the
complete installer.
