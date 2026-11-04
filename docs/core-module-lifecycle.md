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
completed Zygisk runtime. The accepted [continuity policy](rfcs/0005-unified-backend-lifecycle-policy.md)
keeps working backends and excludes automatic cross-family fallback. The existing
writer can stage a device-local kernel choice for the next device boot; it does
not change the running family. Core-package update PendingRestart is a separate
maintenance receipt. Managed Zygisk dispatch/bootstrap remains incomplete.

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
Zygisk hook exposure constrain any explicitly requested transition; installed alternatives are not conflicts
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

The existing-Core replacement command now uses the durable writer, bounded
manager dispatch, staged readback, guarded legacy retirement and post-boot
running-image verification. PendingRestart confirms only the prepared package;
there is no automatic reboot. Missing-Core/incompatible-RPC bootstrap/manager
handoff remains incomplete. See [maintenance](core-maintenance.md).
Compatible client/UI-only APK changes need not imply a Core reinstall; actual
Java hook, runtime and contract changes have separate lifecycle requirements.
A compatible APK-only change requires no device reboot. Preparing a backend is
separate from rebooting; Core never reboots the device automatically.

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

## Managed discovery preparation boundary

`artifact-model::zygisk_projection` defines the closed inert image:
`module.prop`, `vpnhide-projection.json`, `zygisk/arm64-v8a.so` and
`zygisk/armeabi-v7a.so`. The descriptor binds an operation, source commit, exact
raw and normalized manifest digests and both final library identities. The
recipient is exactly `vpnhide_zygisk_runtime` with control2/activation1/projection1.
There is no service, independent updater, installer logic, targets file or hook
authorization in this image. The Core ZIP still carries only its passive pair.

The Root preparation component pins the actual device boot, running Core, enabled source and
checked manager, verifies/copies the final bytes into a sealed archive, and removes
temporary preparation files before admission. Its readback distinguishes a complete
staged tree, complete active tree and metadata-only active placeholder; a placeholder
alone or manager exit0 cannot verify readiness. Symlinks, extra scripts/trees,
foreign targets, writable files and either missing ABI fail closed. These are
filesystem evidence fixtures, not certification of a provider's scan/boot lifecycle.

A journal17 work/dispatch component now enforces persistence before manager spawn,
retains uncertain outcomes without redispatch and provides read-only boot/code
observations. Production boot now binds the exact confirmed active generation
to the loader before probing/selection, with source/recipient directory leases and
exact retained authority readback. Unknown residue records a possible-exposure
fence, and new kernel loads recheck residue around durable admission. This is
read-only code/namespace evidence, not authorization or backend readiness.
The current public chooser is not yet connected to dispatch; its caller
must be the existing writer and bootstrap authorization remains incomplete. New managed recipients
require current-boot journaled authority and exact consumed generation/policy proof;
an empty bootstrap proves code/consumption only, never hooks or behavioral coverage.
Production still refuses new Zygisk choice and current-Zygisk retirement. Production
projection admission, bootstrap, continuity/withdrawal/update and the genuine
mixed-ABI secondary32 worker remain implementation work.
