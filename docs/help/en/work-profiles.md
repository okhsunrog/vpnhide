# Work profiles, clones and second space

**Install and configure VPN Hide in the main profile only.** Its Java hooks run
in the system-wide `system_server`; a copy in each profile is not needed. The app
blocks its UI in secondary profiles. Remove redundant copies if prompted.

## One package, one selection

The main-profile picker gathers installed packages and their UIDs across profiles.
A package present in several profiles has one selection, with profile information
shown in the list. Its roles and hook settings apply to all discovered copies.
**You cannot give different roles to the personal and work copies of the same
package.** A clone with a different package name can be configured separately.

Native capacity counts distinct UIDs, not rows: one package in two profiles usually
uses two slots. VPN Hide reserves its own slot for the main-profile UID only.
See [Limits](capabilities-limits.md).

If a profile's apps are missing or the scan is incomplete, check that the profile
is available, then retry the app-list load. Do not assume that a missing row means
its saved configuration was deleted. Profiles and VPN routing are separate:
check the target's actual VPN/profile arrangement when troubleshooting.
