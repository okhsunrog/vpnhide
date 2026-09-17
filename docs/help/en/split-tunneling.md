# Direct access or through the tunnel

Where your target apps get their network — straight out through your normal
connection, or through the VPN tunnel — is decided in **your VPN client**, not in
VPN Hide. VPN Hide's job is only to make the VPN *invisible* to the apps you
pick. Both routing choices work with it, but they solve different problems, so
decide per app which one you need.

## Scenario 1: the app must not use the VPN, and must not see it

The usual case for banks, government services, payment and delivery apps, and
anything that checks your IP or region. You want the app to behave exactly as it
would on a phone without a VPN.

1. In your VPN client, **exclude the app from the tunnel** (the split-tunnel,
   per-app routing or "bypass" list). Its traffic now goes out through your
   ordinary carrier or Wi-Fi connection, and the service sees your real IP.
2. In VPN Hide, give the app **Java** and **Native** (plus **Apps** if it scans
   for installed VPN apps, **Ports** if it probes localhost). Without this the
   app can still tell that a VPN is *running* on the phone — from the interface
   list, the routes, the Android VPN network callbacks — even though its own
   traffic bypasses it.
3. Save, force-stop the app, reopen it.

What the app sees: a normal connection with your real IP, no VPN interface, no
VPN network. What remains detectable: nothing VPN-related on the device side;
checks unrelated to the VPN (root, device integrity) are outside VPN Hide's job.

Watch the **kill-switch**: if your VPN client blocks traffic that does not go
through the tunnel, the excluded app loses network. Exempt it from the
kill-switch.

## Scenario 2: the app must use the VPN, but must not see it

The case for an app whose traffic you want inside the tunnel — to reach a
blocked service, or to hide your real IP from it — and which nevertheless
refuses to work "while a VPN is active".

1. In your VPN client, keep the app **inside the tunnel**.
2. In VPN Hide, give it **Java** and **Native** (and **Apps** / **Ports** as
   needed). VPN Hide filters what the app can learn locally: the VPN interface
   disappears from the interface list, routes, socket binds and `/proc/net`, and
   the Android network APIs describe the underlying Wi-Fi or mobile network
   instead of the VPN.
3. Save, force-stop the app, reopen it. Check that the app still connects with
   the selected hooks; if one of them breaks its networking, turn that hook off
   for this app in its hook selection.

What the app sees: an ordinary Wi-Fi or mobile connection, while its traffic
actually travels through the tunnel. What remains detectable: the **exit IP** of
your VPN server. If the service checks that IP against known VPN or hosting
ranges, or its geolocation does not match, it can refuse you no matter what the
device reports — VPN Hide hides the VPN *on the device* and cannot change what
the server sees. If the app still refuses after all of this, it does not need
the tunnel after all: move it to scenario 1.

## Where to set it

In a general VPN client, this is the **split tunnel** / **per-app routing**
setting: choose which apps go through the tunnel and which go direct. Put your
target apps wherever you need them per the two scenarios above. Some VPNs route
only a fixed set of apps and offer no per-app choice; then every target app is
in scenario 2.

## Keep VPN Hide itself in the tunnel

For the built-in self-test to have anything to measure, **VPN Hide itself should
be routed through the tunnel** (don't split-tunnel it out). If your VPN routes
only selected apps and won't let you add VPN Hide, the self-test simply can't run
— that's expected, not a failure; judge hiding by whether your target app works.
See [What the self-test checks](what-the-check-proves.md).

## A "block connections without VPN" kill-switch

If your VPN client has a kill-switch (block traffic when the tunnel is down, or
block anything not routed through it), remember it can cut off the target apps
you deliberately routed **direct** (scenario 1). Exclude those apps from the
kill-switch, or they'll lose network whenever it engages.
