# Game accelerators

A game accelerator — UU, Xunyou, Tencent, Biubiu and the like — is itself a VPN.
On Android it builds a VpnService tunnel and routes the games you pick through it,
so to the system it looks like an active VPN and an installed VPN app. Other apps
can react to that.

## Hide the accelerator's VPN from another app

If a bank or another app refuses to run "because a VPN is on" while your
accelerator is active:

- Give **that** app **Java + Native** for network hiding and **Apps** for package hiding.
- Add the accelerator to the hidden VPN-apps list (**Settings → VPN app hiding**,
  or [open it now](vpnhide://hidden-apps)) so package scans don't spot it.

Because accelerators route only the games you select, the app you are hiding from
is off the tunnel anyway — hiding the VPN from it doesn't change its networking.

## Why the self-test can't run

Accelerators may allow only a fixed game list. Core diagnostics does not require
adding VPN Hide to that list: it uses its own bounded fixtures, independently
of the accelerator's routing or whether its VPN is active. Read the recorded
scope and missing evidence in [Diagnostics](vpnhide://diagnostics). A fresh worker
result does not prove that a running game has consumed the same configuration.

## What this is not

This is about hiding the accelerator's VPN from *other* apps. Hiding it from the
game you are accelerating — to get past that game's own anti-cheat — is a
different matter: it can break the game's rules and risk a ban, and against
kernel-level anti-cheat VPN Hide can't promise anything.

Back to [Set up hiding](configure-hiding.md).
