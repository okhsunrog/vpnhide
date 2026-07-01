# portshide

Magisk / KernelSU-Next module that blocks selected apps from reaching
`localhost` ports. Used to hide locally-bound VPN / proxy daemons
(Clash, Sing-box, V2Ray, Amnezia, etc.) from apps that probe for them
via `connect(127.0.0.1, PORT)` / `connect(::1, PORT)`.

## How it works

A small Rust activator reads `/data/system/vpnhide_config.json`, resolves
packages through PackageManager, and installs `iptables` / `ip6tables` rules
inside a dedicated chain `vpnhide_out` / `vpnhide_out6`:

```
iptables -A vpnhide_out -m owner --uid-owner <UID> -d 127.0.0.0/8 -p tcp -j REJECT --reject-with tcp-reset
iptables -A vpnhide_out -m owner --uid-owner <UID> -d 127.0.0.0/8 -p udp -j REJECT --reject-with icmp-port-unreachable
```

...for every UID whose package has `"ports": true` in
`/data/system/vpnhide_config.json`, plus the same for `::1` via `ip6tables`.
The IPv4 rules match the whole `127.0.0.0/8` loopback block, not just
`127.0.0.1`, so a daemon bound to the wildcard `0.0.0.0` stays unreachable on
every `127.x.x.x` alias (`::1` is already the entire IPv6 loopback).
If the app also has a `portPolicy.rules` list, the activator adds `--dport`
matches for those TCP/UDP ports instead of blocking the whole loopback range.
A jump from `OUTPUT` into the dedicated chain is inserted exactly once
(`iptables -C` guarded).

Observer apps receive `ECONNREFUSED` — indistinguishable from a real
closed port. `netd`'s own chains are never touched; our chain lives
beside them and is idempotently re-applied on every config change.

## Install

Pick `vpnhide-ports.zip` in the KernelSU-Next or Magisk manager and
install. Reboot not strictly required — rules apply on next boot via
`service.sh`, or immediately if you manage observers through the VPN
Hide app (it invokes the ports activator via `su`).

## Configuration

Managed by the VPN Hide app (**Hiding** tab -> **Ports** / **P**, settings icon
next to the role label). Direct shell
alternative:

```
# Edit /data/system/vpnhide_config.json:
# "com.example.app": { "java": false, "native": false, "appHiding": false, "ports": true }
# or with ranges:
# "com.example.app": {
#   "ports": true,
#   "portPolicy": {
#     "mode": "custom",
#     "rules": [
#       { "protocol": "both", "start": 1080 },
#       { "protocol": "tcp", "start": 7890, "end": 7892 }
#     ]
#   }
# }
```

Then:

```sh
su -c /data/adb/modules/vpnhide_ports/activator
```

## Why just localhost, and why for selected apps only

- Banking / anti-censorship detection apps probe `127.0.0.1:7890`,
  `127.0.0.1:1080`, `127.0.0.1:8080` etc. Blocking these globally
  would break the VPN client itself (it needs to bind / use them).
  Per-UID REJECT gives surgical control.
- Blocking **all** ports on loopback for observers is still the default and is
  safe for typical observer apps: banks, госуслуги, marketplaces, non-browser
  Yandex apps, VK — none legitimately use localhost.
- Per-app port ranges are available for apps where the full loopback block is
  too broad. Presets are stored as materialized rules, so imported/exported
  configs keep their exact behavior.
- Browsers (Chromium-based) are the only category with some
  legitimate localhost use (dev tools, PWAs). Just don't add them as
  observers.

## Caveats

- Rules are lost on reboot. `service.sh` restores them early in boot,
  waiting for `netd` to finish its setup first (checks
  `bw_OUTPUT` readiness).
- Some Android versions rebuild `OUTPUT` on network state changes.
  Our rules in our own chain survive; only the `OUTPUT -j vpnhide_out`
  jump can be affected. Re-run the activator if needed; the VPN Hide
  app's Save action does this automatically.
- `iptables-legacy` backend expected (default on AOSP 16 as of this
  writing). nftables backend via `iptables-nft` also works — same
  syntax, same kernel effect.

## Uninstall

Via root manager — `uninstall.sh` flushes and removes our chains.
