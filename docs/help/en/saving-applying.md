# When changes take effect

Saving in the picker writes your config and re-runs the active native backend
right away. But a target app that's already running may have **cached** the old
view — its network state, and any VPN check it did at launch. So the reliable way
to apply a change is:

1. **Save** in the app.
2. **Force-stop the target app** (Android Settings → Apps → the app → Force stop)
   and open it again, so it re-reads a fresh, hidden view.

## By backend

- **Java (LSPosed) and Native (kmod / KPM)** — the new config is live after Save;
  restart the target app to clear its cached state.
- **Zygisk** — hooks are installed when the app process starts, so a config
  change **only** takes effect on a fresh launch. Force-stop and reopen the app.

## Things that need a reboot

- **Installing, updating, enabling or disabling a module** (any native backend or
  Ports) — reboot so it loads.
- **Experimental protection** (Hide VPN filesystem paths) on kmod / KPM — reboot.
  On Zygisk it applies after restarting the selected apps instead.

## The Dashboard check is also cached

The self-test result on the Dashboard reflects the last run. After changing
config, restart the target app (and, if needed, your VPN) and re-check — see
[A third-party tester still finds the VPN](tester-finds-vpn.md).
