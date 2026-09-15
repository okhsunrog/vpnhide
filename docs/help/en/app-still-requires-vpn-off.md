# An app still asks you to turn off the VPN

If an app keeps refusing to run "while a VPN is active," work through this before
concluding it's broken.

## Checklist

1. **Right app, right roles.** Give the roles to the app that complains — not
   your VPN client — and give it both **Java** and **Native**. See
   [Set up hiding](configure-hiding.md).
2. **Right profile.** Configure it in your main profile; see
   [Work profiles, clones and second space](work-profiles.md).
3. **Save, then restart the app.** Force-stop the target app and reopen it so it
   re-checks against the hidden view — a running app may have cached the old
   answer. See [When changes take effect](saving-applying.md).
4. **Does it scan for a VPN app?** If it looks at installed packages, turn on the
   **Apps** role and make sure your VPN client is hidden.
5. **Does it probe localhost?** If you run a local proxy/VPN daemon, add the
   **Ports** role.
6. **Zygisk + a banking app?** If your backend is Zygisk and enabling Native made
   it worse, turn Native off for that app and rely on Java.

## When hiding is working but the app still refuses

Run [Diagnostics](vpnhide://diagnostics). If the self-test passes, VPN Hide's
layers are doing their job and the app is refusing for a reason VPN Hide can't
change:

- **Server-side checks.** The service sees your exit IP. If that IP belongs to a
  known VPN/hosting range, it can block you no matter what the device reports —
  route that app **directly** instead of through the tunnel. See
  [A third-party tester still finds the VPN](tester-finds-vpn.md).
- **A detection path outside the active backend's reach.** Diagnostics marks
  these *"Not covered."* A kernel backend, or Experimental protection, may close
  more — see [What it can and can't do](capabilities-limits.md).
- **A cached result.** Restart the app (and, if needed, your VPN) and check again.

If none of that resolves it, [collect a debug report](collect-report.md) so the
exact checks can be inspected.
