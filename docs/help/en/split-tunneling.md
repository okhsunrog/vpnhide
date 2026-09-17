# Direct access or through the tunnel

Where your target apps get their network — straight out through your normal
connection, or through the VPN tunnel — is decided in **your VPN client**, not in
VPN Hide. VPN Hide's job is only to make the VPN *invisible* to the apps you
pick. Both routing choices work with it:

- **Target app routed directly (outside the tunnel).** The app uses your ordinary
  carrier/Wi-Fi connection and sees your real IP, and VPN Hide hides the VPN
  interface from it too. This is the usual choice for banks and services that
  check your IP or region — they see a normal, VPN-free connection.
- **Target app routed through the tunnel.** The app's traffic is tunneled, but
  VPN Hide filters local network information according to the enabled layers.
  It does not change the tunnel's exit IP or guarantee that every detection method
  is covered. Test that the app can still connect with the selected hooks.

## Where to set it

In a general VPN client, this is the **split tunnel** / **per-app routing**
setting: choose which apps go through the tunnel and which go direct. Put your
target apps wherever you need them per the two cases above.

## Keep VPN Hide itself in the tunnel

For the built-in self-test to have anything to measure, **VPN Hide itself should
be routed through the tunnel** (don't split-tunnel it out). If your VPN routes
only selected apps and won't let you add VPN Hide, the self-test simply can't run
— that's expected, not a failure; judge hiding by whether your target app works.
See [What the self-test checks](what-the-check-proves.md).

## A "block connections without VPN" kill-switch

If your VPN client has a kill-switch (block traffic when the tunnel is down, or
block anything not routed through it), remember it can cut off the target apps
you deliberately routed **direct**. Exclude those apps from the kill-switch, or
they'll lose network whenever it engages.
