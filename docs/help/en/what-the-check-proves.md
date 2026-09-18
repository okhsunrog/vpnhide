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
still refuse for other reasons — a server-side check on your exit
IP, an installed-app scan you haven't covered, or a result it cached before you
saved. See [A third-party tester still finds the VPN](tester-finds-vpn.md).

## Scope of the evidence

The probes run as VPN Hide, not inside your bank or detector. Per-app hook settings,
UID/profile routing, API choice and callback timing can differ. Passing does not
rule out a bug affecting another app. Test that app after Save and force-stop,
and include its reproduction steps in a report if it still detects the VPN.

A steady-state callback probe also cannot certify every Wi-Fi/mobile/offline
transition or every combination of callback registrations. Those require separate
transition tests. For current, retained and interrupted results, see
[Result meanings](check-result-meanings.md).

## Why the check count isn't the hook count

The Diagnostics list has more entries than the backend has hooks (for example
15 native checks against 10–11 kernel hooks). That's expected: a check is a *way
an app could detect the VPN*, and one hook often covers several of them, while
some checks are covered by the kernel or SELinux with no hook at all. The
self-test measures each vector's outcome, not one probe per hook. The hook count
lives on the [Statistics](statistics.md) screen.
