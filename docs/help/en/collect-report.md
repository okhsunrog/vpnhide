# Collect a debug report

When you report a problem, a debug bundle lets the author see what your device
actually did — far more useful than a description alone.

## How

1. Turn on **Debug logging** in **Diagnostics** (or Settings → Debugging). It's
   off by default. Normal apps can't read logcat, so this isn't a stealth risk.
2. Reproduce the problem: with a VPN up, cold-start the target app so its checks
   run again.
3. In **Diagnostics**, use **Export** to build the debug bundle
   (`vpnhide_debug_*.zip`).
4. Share the ZIP in the [Telegram group or a GitHub issue](vpnhide://diagnostics).
   Turn Debug logging back off afterwards.

## What's in it, and what isn't

The bundle contains the diagnostic report, module states, and the debug log —
the technical state needed to diagnose hiding. Review it before sharing if you're
unsure; it's meant for triage, not for collecting personal data. A capture taken
without a VPN, or while VPN Hide isn't routed through the VPN, is incomplete —
the app tells you when that's the case.
