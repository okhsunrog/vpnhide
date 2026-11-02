# A module is installed but not active

The Dashboard distinguishes installed files, backend selection, confirmed
application and measured protection. Follow the specific current error.

## Common causes

- **Core missing or incompatible:** install the full Core ZIP and matching APK.
- **Restart pending:** complete the requested reboot, then unlock. A provider may
  still hold an older payload until its lifecycle boundary.
- **Java scope missing:** enable VPN Hide and System Framework in LSPosed / Vector.
- **KernelPatch unavailable/authentication failed:** check [KPM requirements](kpm-install.md).
- **Old separate modules or independently loaded backends:** follow migration
  guidance; retain configuration and verify the replacement before retirement.
- **Missing/corrupt payload:** use the complete paired Core installer.
- **Unsupported kernel or incomplete checks:** this is not resolved by guessing a
  GKI variant, reinstalling a separate backend ZIP or enabling an external VPN.

All alternatives bundled in Core are expected and do not conflict merely by
being present. If the issue persists, [collect a report](collect-report.md).
