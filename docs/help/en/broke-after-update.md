# It stopped working after an update

An Android update, a root-manager update, or a VPN Hide update can leave the
pieces out of step for one reboot.

## Check these

- **Version mismatch.** The Dashboard shows the app version and the *running
  module version*. After installing matching components, **reboot** so the new
  code loads. If the warning remains, check which component still needs updating;
  a version mismatch does not always mean only a reboot is missing.
- **Kernel changed.** A system update can change the kernel. A kernel module
  built for the old kernel may fail to load ("Unknown symbol", "not active").
  Reinstall the native backend that matches the new kernel, or switch to one the
  Dashboard now recommends.
- **LSPosed scope dropped.** Some updates reset module scopes. Re-check that
  **System Framework** is still in VPN Hide's scope.
- **A target app updated.** The app may have added a new detection method. Open
  it, and if it complains again, re-check its roles — you may need **Native** or
  **Apps** where **Java** alone used to be enough.

After a reboot the Dashboard should read all modules **Active** again. If a
backend won't load on the new kernel, see
[A module is installed but not active](module-not-active.md).
