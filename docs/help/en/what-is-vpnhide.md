# What VPN Hide does

VPN Hide makes a running VPN **invisible to the apps you choose** — banks,
government services, marketplaces and other apps that refuse to work when they
detect a VPN. It does not turn the VPN off; your traffic keeps flowing. It only
hides the *signs* of the VPN from the apps you point it at.

## When it helps

- An app shows "turn off your VPN" or silently fails to load while your VPN is on.
- An app checks for an **installed** VPN client and refuses to run if it finds one.
- An app probes localhost for a VPN/proxy daemon.

You configure this per app, on the app you're hiding *from* — never on the VPN
client itself. See **[Set up hiding](configure-hiding.md)**.

## What it can't do

VPN Hide changes what apps *see on the device*, not your network identity:

- **Your external IP doesn't change.** A server still sees the VPN exit IP. If a
  service blocks by IP or does its checks server-side, hiding on the device can't
  help — that's outside the phone.
- **Root is required.** Java/Apps use LSPosed, Native uses one native backend,
  and Ports uses its own module. See [Requirements](requirements.md).
- Your VPN client decides whether a target goes through the tunnel or directly.
  VPN Hide supports both arrangements, with coverage depending on the enabled
  layers. See [Routing](split-tunneling.md).
