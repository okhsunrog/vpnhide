# What each result means

Each check reports one outcome per detection vector. On the Diagnostics screen
the labels are:

- **OK** — the app can't see the VPN through that vector, but root can. The active
  backend is hiding it. This is the result you want.
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
