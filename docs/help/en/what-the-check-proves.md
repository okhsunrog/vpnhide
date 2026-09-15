# What the self-test checks

The **Diagnostics** screen runs VPN Hide's own checks: for each detection vector
it compares what a target app would see against the real, root-observed state.
If the app-side view no longer shows the VPN while root still does, that vector
is **hidden**.

## What a pass means

A passing run means the hiding layers are actually working on your device, for
the vectors tested. Because the checks measure the same hooks that hide the VPN
from your target apps — the same code path, not a weaker stand-in — a pass is a
real signal that hiding works, which is exactly why the Dashboard can say
"VPN hidden".

## What it needs to run

- A **VPN must be up** — otherwise there's nothing to hide, and the checks have
  nothing to measure.
- **VPN Hide must be routed through that VPN.** If it isn't (some app-specific
  VPNs and game accelerators only route selected apps), the app-side probe
  wouldn't see the VPN even without hiding, so the check can't run. The screen
  says so; it's a state to fix, not a failure.

## What it does not prove

A pass doesn't guarantee a *specific* third-party app will accept you. An app can
still refuse for reasons outside the device — a server-side check on your exit
IP, an installed-app scan you haven't covered, or a result it cached before you
saved. See [A third-party tester still finds the VPN](tester-finds-vpn.md).
