# Game accelerators

A game accelerator — UU, Xunyou, Tencent, Biubiu and the like — is itself a VPN.
On Android it builds a VpnService tunnel and routes the games you pick through it,
so to the system it looks like an active VPN and an installed VPN app. Other apps
can react to that.

## Hide the accelerator's VPN from another app

If a bank or another app refuses to run "because a VPN is on" while your
accelerator is active:

- Give **that** app the **Java + Native** roles, so it can't see the VPN.
- Add the accelerator to the hidden VPN-apps list (**Settings → VPN app hiding**,
  or [open it now](vpnhide://hidden-apps)) so package scans don't spot it.

Because accelerators route only the games you select, the app you are hiding from
is off the tunnel anyway — hiding the VPN from it doesn't change its networking.

## Why the self-test can't run

Accelerators usually route a fixed game list, not arbitrary apps, so they often
won't let you add VPN Hide to their tunnel. When that's the case, the Overview
self-test can't run — it reports that VPN Hide isn't routed through the VPN. That
is expected, not a failure: judge hiding by whether your target app works. If your
accelerator has a global mode, it routes VPN Hide too and the self-test runs.

## What this is not

This is about hiding the accelerator's VPN from *other* apps. Hiding it from the
game you are accelerating — to get past that game's own anti-cheat — is a
different matter: it can break the game's rules and risk a ban, and against
kernel-level anti-cheat VPN Hide can't promise anything.

Back to [Set up hiding](configure-hiding.md).
