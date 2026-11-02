# Core module lifecycle

Users install the full universal `vpnhide-core.zip` with a compatible APK. The
Magisk-format main module is `vpnhide_core`; Core owns selection, policy and effects.
See [architecture](core-architecture.md), [maintenance](core-maintenance.md) and
[current work](../ROADMAP.md). A base daemon ZIP is only an internal build input.

## Entrypoints and layout

`core/module/service.sh` directly execs `vpnhide-core service`. `customize.sh` only
places the selected ARM ABI executable and sets its mode. `sepolicy.rule` is
manager-applied policy. Recovery installer shims are installation adapters. The
scaffold check enforces these boundaries; backend activation is not shell logic.

The full package contains both daemon ABIs before customization, a verified payload
manifest, seven ko candidates, KPM and one passive Zygisk ABI pair. The current
package cutover has no main-Core `zygisk/` discovery directory; it is not a
completed Zygisk runtime. Final publication/transition policy is
[reopened](rfcs/0003-conditional-zygisk-publication.md); no managed projection,
automatic next-boot preparation or new PendingRestart workflow is implemented.

## Service startup

The Rust service preset establishes production paths, ownership and bounded logging
before threads start. Instance guards protect durable writer identity. Core loads
canonical policy and journal; unknown effects remain held rather than replayed.
The app socket supervises app-directory availability and can recover independently
of the daemon. No running Activity is required.

Native preparation waits for PackageManager/complete UID observations outside the
writer. The verified catalog, app inventory, backend choice, payload identity and wire
policy are frozen before journaled effects. Core adopts an applicable builtin or
selects eligible bundled candidates. Unknown loads, policy gaps and possible
Zygisk hook exposure constrain fallback; installed alternatives are not conflicts
merely because their files coexist. Legacy installed/runtime evidence remains
isolated for migration/recovery.

Native/Ports boot application and backend observation have their own bounded work.
Autonomous diagnostics owns its fixtures and background admission; external VPN
presence and APK split-tunnel membership are not readiness prerequisites.

## Updates

A manager-staged replacement is not the active/running generation. Installers may
update the live `module.prop` while staging; pin executable/catalog identities and
verify staged bytes independently. Interrupted/unknown effects keep durable holds.
Do not remove old modules before the verified replacement is ready.

The common repair/install/update/bootstrap executor is still incomplete. Current
package authentication/staging/image fences do not constitute that executor.
Compatible client/UI-only APK changes need not imply a Core reinstall; actual
Java hook, runtime and contract changes have separate lifecycle requirements.
Reboot counts and new backend hints remain under review, not a blanket policy.

## Disable and removal

A manager-disabled Core may no longer start. Existing kernel hooks, process hooks
and Ports rules are not unloaded merely by disabling or deleting files. Current
Core has no independent backend activator that repairs it from another module.

Uninstall does not authorize deleting the private journal or attempt budgets.
Explicit reset uses the Core writer and retained operation identity; unavailable
Core does not grant an offline wipe. Future managed-discovery cleanup must address
Core disable/remove and sudden power loss as part of the reviewed lifecycle.

## Fixtures and production boundaries

`service` and internal `service-worker` establish the compiled production preset
and clear inherited `VPNHIDE_*` overrides. Plain executable runs are not sandboxes:
fixture callers must explicitly isolate config, journal, socket and effect paths.
App socket failure retries the endpoint rather than replaying boot effects.

Real manager staging, SELinux, provider catalogs and device reboot acceptance remain
separate from host tests. Never use the manager's global module-removal command
for a single VPN Hide component.
