# When changes take effect

After choosing roles in **Hiding**, tap **Save** and wait for the outcome. Then
force-stop the target app in Android settings and reopen it: apps can cache an
old VPN result even after hiding has been applied.

## Saved is not always applied

- **Success:** the configuration was saved and the requested apply steps completed.
- **Saved, but applying failed:** the choices are stored, but one or more layers
  may still use the previous configuration. Read the error, fix the cause (for
  example, root access or a missing module), then use the offered apply-again action.
- **Outcome unknown / changes paused:** VPN Hide could not confirm how the previous
  operation ended. Use **Check again**. Do not repeatedly import or change settings
  to work around it. If checking cannot resolve it, [collect a report](collect-report.md).
- **Unsaved edits:** changes in the picker are a draft until Save succeeds. Use
  **Discard edits** to return to the saved selection. If a conflict is reported,
  review the refreshed configuration before making your choice again.

## Restart an app or reboot the phone?

| Change | What to do |
|---|---|
| Java / Apps roles or kernel Native targets and hooks | Save successfully; force-stop the target app to discard its cached results |
| Zygisk targets or hooks | Save, then force-stop and reopen the target app; its hooks are installed when its process starts |
| Ports targets or ranges | Save successfully; firewall rules change without rebooting |
| Install, update, enable or remove a module | Reboot to complete the root manager's operation |
| Install or update the VPN Hide APK, or change its LSPosed scope/enabled state | Reboot: hooks already loaded in `system_server` keep running until then |
| Experimental filesystem protection on a kernel backend | Reboot; follow the pending-state message |
| Experimental filesystem protection on Zygisk | Restart the selected apps |

Technically, Save runs an activator to deliver configuration; it does not unload
and restart the kernel backend.

## Dashboard refresh

VPN Hide refreshes state when you return to the app. If the Dashboard still shows
an earlier VPN state, tap **Retry** to request a fresh check. Restarting the target
app alone does not refresh or prove its hiding; see
[What the self-test checks](what-the-check-proves.md).
