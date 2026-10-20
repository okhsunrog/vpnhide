# A module is installed but not active

The Dashboard shows each module's state. "Installed but not active" almost always
has one of a few concrete causes — the on-screen message names the exact one.

## Common causes

- **Missing KernelPatch runtime.** “The kernel is not patched” means VPN Hide could not detect a working KPM runtime. Installing `vpnhide-kpm.zip` does not provide it. See [where to download KPatch-Next-Module and how to install the runtime](kpm-install.md).
- **VPN Hide Core missing.** Every backend is loaded by the core module
  (`vpnhide-core.zip`). Without it nothing loads; the app shows **VPN Hide Core is
  not running** at startup. Install it from the same release and reboot.
- **Reboot to activate.** A freshly installed native module only loads after a
  reboot: core picks and loads the backend once per boot. Restart the device.
- **Missing LSPosed scope.** The Java layer needs **System Framework** in
  VPN Hide's scope. Add it in LSPosed / Vector and reboot.
- **More than one native backend.** Core loads a backend only when it is the
  only one: with two installed (or one installed on a kernel with VPN Hide built
  in) it loads **none** and the Dashboard lists them. Uninstall the extras and
  reboot.
- **KPM without a SuperKey.** APatch/FolkPatch couldn't load the KPM with the
  trusted token. Save your SuperKey in **Settings → Security**; core then loads
  the waiting KPM without a reboot.
- **Obsolete 1.x module.** A **VPN Hide (Ports)** or **VPN Hide (Built-in kernel)**
  module from 1.x is still installed. 2.0 replaced both with the core module and
  their old boot scripts interfere with it: uninstall it and reboot.
- **Corrupted install.** A missing payload (the `.ko`, `.kpm` or Zygisk library)
  means the module ZIP didn't install cleanly. Reinstall the full ZIP from the latest release
  through your root manager's Modules screen — don't extract the inner file.
- **Unsupported kernel.** The KPM has no validated offsets for this kernel
  family. Use the Zygisk backend instead.

Whatever the message says, do the action it names, then tap **Refresh** (or
reboot when it asks). If it still won't activate, collect a
[debug report](collect-report.md).
