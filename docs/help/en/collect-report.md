# Collect a debug report

**Settings → Debugging → Debug export** builds one file (`vpnhide_debug_*.zip`)
with the diagnostics needed to see what your device actually did — far more useful
than a description alone.

## How

1. Reproduce the problem: with a VPN up, cold-start the target app so its checks
   run again.
2. Open **Settings → Debugging → Debug export**.
3. Choose what to include. **Verbose logs** (dmesg, logcat, probes) is usually
   what a bug report needs. The installed-app list and kernel image are off by
   default — add them only if the developer asks.
4. Tap **Export** and share the ZIP in the Telegram group or a GitHub issue.

For continuous logs across the cold start, turn on **Debug logging** first in
**Settings → Developer**, reproduce, then export. The capture in Debug export
enables logging on its own, so you usually don't need to — turn it back off after.

## What's in it, and what isn't

The bundle carries the diagnostic report, module states and the logs you picked —
the technical state needed to diagnose hiding, not a way to collect personal data.
The installed-app list stays out unless you enable it. A capture taken without a
VPN, or while VPN Hide isn't routed through the VPN, is incomplete — the app says
so.
