# It stopped working after an update

An Android, root-manager or VPN Hide update can change compatibility and loaded
versions. Check the actual reported state before changing the installation.

## Check these

- **Version mismatch:** check compatible APK/Core artifacts and the actual
  running generation. A staged Core replacement needs its lifecycle boundary;
  a compatible client/UI-only APK update does not imply a Core reinstall.
- **Kernel changed:** Core derives candidates from the verified bundled catalog.
  Keep the compatibility/load error. Do not manually choose a GKI ZIP or install
  a separate backend archive.
- **LSPosed scope changed:** verify that **System Framework** remains in VPN Hide's
  scope. A newly installed APK may differ from code already loaded there.
- **Target app updated:** review roles and restart that app after Save; it may
  have added a detection method.

Rebooting does not guarantee that every backend becomes active. Read the specific
setup, delivery and measured-coverage status. See
[A module is installed but not active](module-not-active.md).
