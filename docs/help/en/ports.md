# Hide localhost ports (Ports)

Some VPN/proxy clients listen on local ports, for example `127.0.0.1:7890`.
An app can probe them even when the VPN interface is hidden. **Ports** blocks
these connections for the target app; it does not stop the proxy itself.

## Set it up

Ports is part of the **VPN Hide Core** module (`vpnhide-core.zip`), which every
installation already has; there is nothing extra to install. A **VPN Hide (Ports)**
module left over from 1.x must be uninstalled — the Dashboard shows an error while
it is still there.

1. In **Hiding**, enable **Ports** for the app doing the probing.
2. Use the settings icon beside Ports if you want to choose port ranges, then **Save**.
3. Wait for a successful apply and reopen the target app.

Changing roles or ranges applies firewall rules on Save without a reboot; core
also re-applies them at every boot. If applying fails, follow [Saving and applying](saving-applying.md).

## Coverage and side effects

The default blocks all TCP/UDP ports on IPv4 loopback `127.0.0.0/8` and IPv6 `::1`
for the selected UID. TCP is rejected with a reset; UDP uses a port-unreachable
response. Other UIDs are unaffected; packages sharing a UID also share the block.

Any app can legitimately use a local service. Blocking all ports may break such
features, including browser/dev tools and local integrations. If that happens,
disable Ports for the app or narrow the rules to the proxy ports you need to hide.
This does not cover every way to discover a local service, such as Unix sockets
or a service reachable on another network address.

## Stop blocking

For one app, turn off its Ports role and Save successfully. The rules live in the
kernel's firewall until reboot: uninstalling the core module stops core from
re-applying them, but only a reboot clears rules that are already loaded.
