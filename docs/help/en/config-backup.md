# Back up, restore and transfer your config

Your whole setup — which apps have which roles, port rules, options — lives in a
single canonical JSON config. You can save it, restore it, or move it to another
device from **Settings → Configuration → Config backup**.

## Export and import

- **Export JSON** — saves the current config to a file you choose.
- **Import JSON** — loads a config from a file. **Import replaces** the current
  config (it doesn't merge) and reapplies the installed modules right away.

Keep an exported copy before a big change or a clean reflash, so you can get your
setup back with one import.

## Moving to another device

1. On the old device, **Export JSON**.
2. On the new device, install VPN Hide and the same modules you use.
3. **Import JSON** the file you exported.

Roles apply to apps by package name, so any target app you don't have installed
on the new device is simply carried in the config until you install it.

## Importing an old (pre-1.0) configuration

If VPN Hide finds a configuration written by a version **before 1.0** and you've
already set some apps up, it won't import it silently — it shows a banner and, in
**Settings → Pre-1.0 configuration**, offers three choices:

- **Merge** — keep everything configured now and add the old roles on top.
- **Replace** — drop the current app list and use the old one instead (your
  auto-hide settings and VPN Hide's own entry are kept either way).
- **Delete** — remove the old files without reading them; pick this if you've
  already redone your setup and just want them off the device.

All three **delete the old files**, and that can't be undone.

## What this backup does not contain

The JSON is a backup of hiding configuration: package roles, hook overrides, port
policies, hidden-package choices and canonical options (including debug flags).
It is not a complete app/device backup. It does not include installed modules,
your VPN client's connections or credentials, runtime counters/logs, root grants,
LSPosed scope, or local UI preferences such as theme/language. The actual APatch
SuperKey is stored separately and is **not** exported, even if the remember-key
option is present. Set it again on the new device if needed.

Choose modules compatible with the **new device** rather than copying an old
kernel ZIP blindly. After import, check the application outcome, module states
and UID capacity, then restart targets. A saved configuration can exist even if
applying it failed; see [Saving and applying](saving-applying.md).

The file includes configured package names. Review it before sharing publicly.
Exporting a plain package list in Settings is useful for other tools, but is not
a substitute for the JSON backup: it does not preserve roles and hook/port options.
