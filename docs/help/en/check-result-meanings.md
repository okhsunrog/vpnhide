# What each result means

Each check reports one outcome per detection vector.

- **Hidden** — the target-app view no longer shows the VPN, while root still
  does. The layer is working for that vector.
- **Detected** — the app can still see the VPN through that vector. Hiding didn't
  cover it. Enable the role or backend that handles it (often **Native** where
  **Java** alone isn't enough), Save, and restart the app.
- **Covered by SELinux** — the vector is blocked by the system's SELinux policy
  rather than by a VPN Hide hook. It's effectively hidden on this device, but not
  something VPN Hide is doing — so it can vary by ROM.
- **Not measured** — the check couldn't run. Usually a gate: the VPN is off,
  VPN Hide isn't routed through the VPN, or the app just enabled hiding for
  itself and needs a restart. Fix the gate and re-run.

## Gates you might see instead of results

- **VPN is off** — turn the VPN on and re-check.
- **Not routed through the VPN** — see
  [What the self-test checks](what-the-check-proves.md).
- **Restart VPN Hide** — you just added VPN Hide as a target; force-stop and
  reopen it so its own hooks apply, then re-check. No reboot needed.
