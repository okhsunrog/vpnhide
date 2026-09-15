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
- **Work without root.** It needs a native backend plus LSPosed; without them
  there are no hooks to install.
- **Defeat a game's own anti-cheat.** Hiding a VPN from the very game you're
  playing to get past its checks is a different matter, and not something VPN
  Hide promises — see [Game accelerators](game-accelerators.md).

## Designed for split tunneling

VPN Hide is meant to be used *with* split tunneling: the apps you hide from run
*off* the VPN, so changing what they see about the network doesn't change how
they actually reach the internet. That's why turning hiding on per app is safe.
