# Hide localhost ports (Ports)

Some VPN and proxy clients run a local daemon — Clash, Sing-box, V2Ray, Amnezia
and the like — listening on a `localhost` port such as `127.0.0.1:7890`. An app
can probe those ports with `connect(127.0.0.1, PORT)` to guess a VPN or proxy is
running, even when the interface itself is hidden. The **Ports** role closes that
gap for the apps you pick.

## What it does

For an app you mark as a Ports target, VPN Hide blocks its access to the
loopback address. Probes come back as *connection refused* — indistinguishable
from a genuinely closed port. The block is **per-app**: your VPN client and the
rest of the system still use localhost normally.

By default it blocks the whole loopback range for that app, which is safe for
typical observers — banks, government and marketplace apps, and similar — none of
them legitimately use localhost. **Browsers are the exception** (dev tools, PWAs
and installed web apps do use localhost), so simply don't add a browser as a
Ports target.

## Turn it on

1. Install the **Ports** module (`vpnhide-ports.zip`) through your root manager's
   **Modules** screen. A reboot isn't strictly required — the app re-applies the
   rules when you Save.
2. In the app's **Hiding** tab, give the app the **Ports** / **P** role (the
   settings icon next to the role label).
3. Save. The rules apply immediately.

## Narrowing it to specific ports

If blocking the whole loopback range is too broad for an app, the settings icon
next to the Ports role also lets you set **specific port ranges** to block
instead. These are saved as exact rules, so an exported or imported config keeps
the same behavior.

## Removing it

Uninstall the Ports module from your root manager's Modules screen. Note that the
firewall rules are re-applied at each boot and on every Save; removing the role
(or the module) and saving clears them.
