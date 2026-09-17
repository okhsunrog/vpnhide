# What each result means

Each check reports one outcome per detection vector. On the Diagnostics screen
the labels are:

- **OK** — this probe found the expected hidden/consistent app-side view. For
  root-differential checks, root still sees the signal that the app cannot see.
  The result applies to this probe, not to every possible detector.
- **Leak** — the app can still see the VPN through that vector. Hiding didn't cover
  it. Enable the role or backend that handles it (often **Native** where **Java**
  alone isn't enough), Save, and restart the app.
- **SELinux** — the app's read was blocked by the system's SELinux policy, not by a
  VPN Hide hook. Effectively hidden on this device, but not something VPN Hide is
  doing — so it can vary by ROM.
- **Nothing to hide** — there's nothing to leak here: root sees nothing either.
- **Not covered** — the active backend doesn't cover this vector. A kernel backend
  covers more than Zygisk; switch backends if it matters.
- **No data** — the check couldn't measure. Usually a gate: the VPN is off,
  VPN Hide isn't routed through the VPN, or VPN Hide needs a restart. Fix the gate
  and re-run.

## Gates you might see instead of results

- **VPN is off** — turn the VPN on and re-check.
- **Not routed through the VPN** — see
  [What the self-test checks](what-the-check-proves.md).
- **Restart VPN Hide** — you just added VPN Hide as a target; force-stop and reopen
  it so its own hooks apply, then re-check. No reboot needed.

## Current state and earlier results

**Checking** means an observation or self-test is in progress. A failed or interrupted
attempt is not a pass. The screen can retain earlier measurements and explain why
the latest attempt did not finish; read that notice before interpreting the rows.

If routing could not be determined, it is **unknown**, not “VPN is off”. Check root
access and retry. If a configuration change is applying, wait; if it failed or its
outcome is unknown, resolve that first using [Saving and applying](saving-applying.md).

A single **No data** result can also mean a probe failed or returned incomplete
information. Follow its detail; do not treat it as either a leak or a successful
hide. Conversely, a confirmed leak does not disappear because another probe failed.
