# Security and privacy

VPN Hide reduces supported local VPN signals for selected apps. It does not
provide anonymity, hide root, or guarantee acceptance by an app or its server.

## What must be trusted

You trust the Android system, kernel, root manager, hooking framework and the
modules you install. VPN Hide targets checks made by ordinary apps; it does not
protect you from malicious root software or a compromised system. Hiding VPN
signals also does not guarantee that VPN Hide or its hooks cannot be detected.

## Hiding does not verify routing

A passing diagnostic result covers the measured checks in VPN Hide's process.
It does not prove complete hiding in another app or that its connections stayed
in the VPN. Java hooks may return a physical network; explicitly binding to it
can allow direct access when Android and VPN policy permit bypass.

If you require the tunnel, keep the corresponding VPN blocking policy enabled
and verify the target app's connections. Do not infer routing from an IP check
in another app. See [Direct access or through the tunnel](split-tunneling.md#scenario-2-the-app-must-use-the-vpn-but-must-not-see-it)
and [What the self-test checks](what-the-check-proves.md).

## Failures and partial protection

An installed module is not proof that all its hooks are active. Missing hooks,
failed activation and stale configuration can leave gaps. VPN Hide does not
block all traffic when hiding fails. A permission or SELinux denial is also not
proof that a hiding hook worked. Check the reported status and
[check result meanings](check-result-meanings.md).

## Saved credentials

On APatch/FolkPatch, saving a SuperKey stores it across app and device restarts
in `/data/adb/vpnhide/superkey`, owned by root with mode `0600`. The app offers
Save and Clear, not a session-only mode. Root remains able to read the file.
The key is separate from configuration JSON exports. A saved key is unnecessary
when the trusted `su` token works. See [SuperKey storage](kpm-install.md#superkey-storage).

## Developer access and reports

Agent Control is optional and off by default. Its local HTTP bridge requires a
bearer token and can control configuration. The token is stored in app-private
files and logged for host tools. Anyone who obtains the token and can reach the
bridge can control it; localhost does not identify a particular app. Forwarding
can make the bridge reachable beyond the phone, but still requires the token.
Enable it only for intended tooling sessions and turn it off when finished.

Debug reports can contain app lists, network details, logs and optional kernel
images. They are not anonymous. Redaction covers specific fields, not every
possible secret inside arbitrary logs. Review selected contents before sharing:
[Collect a debug report](collect-report.md).
