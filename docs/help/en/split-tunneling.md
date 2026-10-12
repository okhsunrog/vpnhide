# Direct access or through the tunnel

Configure per-app routing in **your VPN client**. VPN Hide filters supported
VPN signals available to selected apps. These are separate tasks: a successful
hiding check does not establish the route of an app's connections.

## Scenario 1: the app must not use the VPN, and must not see it

Choose this when the app should connect directly over your carrier or Wi-Fi.
The service will see that connection's public IP.

1. In your VPN client, **exclude the app from the tunnel** using split tunneling,
   per-app routing or the bypass list.
2. In VPN Hide, enable **Java** and **Native** for the app. Add **Apps** if it
   scans for installed VPN apps and **Ports** if it probes localhost.
3. Save, force-stop the app, reopen it.

VPN Hide filters supported local signals. It cannot guarantee complete
invisibility: results depend on the backend, installed hooks, settings and the
app's own checks. See [Capabilities and limits](capabilities-limits.md).

If the excluded app loses network access, check the
[block-without-VPN settings](#a-block-connections-without-vpn-kill-switch).

## Scenario 2: the app must use the VPN, but must not see it

Choose this when the app needs the VPN to reach a service or the service must
not see your ordinary connection's public IP.

1. In your VPN client, keep the app **inside the tunnel**.
2. In VPN Hide, enable **Java** and **Native**, plus **Apps** / **Ports** if needed.
3. Save, force-stop the app, reopen it.
4. Check both app compatibility and the actual route of its connections.
   VPN Hide's self-test does not verify another app's traffic route.

**VPN Hide does not guarantee that traffic stays in the tunnel.** Java hooks can
return a physical Wi-Fi or mobile network in place of the VPN network. If the
app explicitly binds a connection to that network, it may connect directly when
Android and the VPN policy permit it. When bypass is prohibited, substituting
the network does not remove that restriction; a connection may also fail.
The hiding status alone cannot distinguish these outcomes.

If direct access is unacceptable, do not disable VPN blocking to make the app
work. Verify routing with your VPN, Android version and target app, including
VPN reconnections. Checking an IP in a separate browser does not verify the
target app's connections. See [Security and privacy](security-and-privacy.md).

Even when traffic stays in the tunnel, the server sees the **VPN exit IP** and
may reject it. VPN Hide cannot change that IP. An app refusing to work does not
establish the cause or mean it no longer needs the tunnel. Continue with
[App still asks to disable VPN](app-still-requires-vpn-off.md).

## Where to set it

Look for **split tunneling** or **per-app routing** in your VPN client. Some
clients route only a fixed set of apps and offer no choice. Check the client's
actual behavior; VPN Hide does not add routing controls to it.

## Keep VPN Hide itself in the tunnel

**VPN Hide itself must be routed through the tunnel** for its self-test. If you
cannot include it, a full self-test is unavailable. A working target app is
useful compatibility evidence, but does not prove complete hiding or tunnel
retention. See [What the self-test checks](what-the-check-proves.md).

## A "block connections without VPN" kill-switch

Your VPN client may have its own block for traffic outside the tunnel. Android
also has a separate system setting to block connections without VPN. Their
behavior and support for exceptions differ.

A block may leave an intentionally excluded app without network access. Allow
direct access only if that is your goal in scenario 1 and your VPN settings
support it. If the app must stay in the tunnel, keep blocking enabled and
investigate compatibility: disabling protection changes your routing policy,
not the effectiveness of hiding.
