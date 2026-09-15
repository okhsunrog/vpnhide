# A module is installed but not active

The Dashboard shows each module's state. "Installed but not active" almost always
has one of a few concrete causes — the on-screen message names the exact one.

## Common causes

- **Reboot to activate.** A freshly installed native module only loads after a
  reboot. Restart the device.
- **Missing LSPosed scope.** The Java layer needs **System Framework** in
  VPN Hide's scope. Add it in LSPosed / Vector and reboot.
- **Two native backends at once.** The kernel module and the KPM hook the same
  kernel functions; keep only one installed. Having both can even freeze the
  device — uninstall one.
- **KPM without a SuperKey.** APatch/FolkPatch couldn't load the KPM with the
  trusted token. Save your SuperKey in **Settings → Security**, then reboot.
- **Corrupted install.** "Activator missing / not executable" means the module
  ZIP didn't install cleanly. Reinstall the full ZIP from the latest release
  through your root manager's Modules screen — don't extract the inner file.
- **Unsupported kernel.** The KPM has no validated offsets for this kernel
  family. Use the Zygisk backend instead.

Whatever the message says, do the action it names, then tap **Refresh** (or
reboot when it asks). If it still won't activate, collect a
[debug report](collect-report.md).
