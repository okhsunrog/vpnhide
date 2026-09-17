# What it can and can't do

## What it can do

Hide the *on-device signs* of a VPN from the apps you choose — the network
signals apps read through Java APIs, the kernel and libc; the presence of an
installed VPN client; and localhost VPN/proxy ports. Different apps probe
different things, and VPN Hide has a role for each: **Java**, **Native**,
**Apps**, **Ports**.

## What it can't do

- **Change your external IP.** A server sees the VPN's exit IP. Anti-fraud and
  geo-checks that run server-side, or key off the IP, are outside the phone — the
  device can't hide that.
- **Beat server-side or account-level detection.** If a service decides from its
  own backend that you're on a VPN, on-device hiding won't change the answer.
- **Hide that you're rooted or on a custom ROM, or make an app pass Play
  Integrity.** VPN Hide hides the VPN and nothing else. An app that refuses
  because it detects root or fails attestation needs a different approach — that
  isn't what this tool does.
- **Work without root.** Root is required. Java/Apps need LSPosed; Native needs
  a native backend; Ports needs its module. You can use fewer layers, with less coverage.
- **Give Native to an unlimited number of apps.** The native backend protects up
  to **159 app UIDs** (it counts UIDs, not apps, and reserves one slot for VPN
  Hide itself). The picker prevents selections/saves over its UID budget. Reduce
  the Native selection if it reports the limit. An imported configuration or new
  profile can also exceed runtime capacity; do not ignore an apply/capacity warning
  or assume every saved target is protected. An app cloned into a work profile or Second Space counts
  twice — see [Work profiles](work-profiles.md). The Java and Apps roles aren't
  capped this way.
- **Defeat a game's own anti-cheat.** Hiding a VPN from the very game you're
  playing to get past its checks is a different matter, and not something VPN
  Hide promises — see [Game accelerators](game-accelerators.md).

## Routing is a separate choice

Targets can be inside or outside the tunnel. VPN Hide changes the local view,
not your VPN client's routing rules or the public IP. Start with the roles you
need, check the target's actual behavior, and undo a change if it breaks a feature.
See [Direct or through the tunnel](split-tunneling.md).
