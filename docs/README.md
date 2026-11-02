# VPN Hide documentation

[ROADMAP.md](../ROADMAP.md) is the single current work index. It distinguishes
implemented code, remaining implementation, decisions under review and device gates.

## User guide

[`help/`](help/README.md) contains the current EN/RU/ZH user instructions. These same
article IDs are shipped in the APK by `syncHelpAssets`. Start with
[first installation](help/en/first-install.md) and [configuration](help/en/configure-hiding.md).
Users install one full Core ZIP with a compatible APK; separate producer backend
ZIPs and daemon-only base archives are internal build inputs.

## Current architecture and reference

| Contract | Scope |
| --- | --- |
| [Core architecture](core-architecture.md) | Ownership, single writer, typed commands, read replicas and recovery |
| [IPC authorization](core-ipc-auth.md) | Socket identity, client/server peer credentials, resource bounds |
| [Module lifecycle](core-module-lifecycle.md) | Actual service/startup, disable/update/removal and unimplemented boundaries |
| [Maintenance](core-maintenance.md) | Implemented authenticated preparation/image fences; executor work still pending |
| [App transitions](app-state-transitions.md) | Current Core-session, draft, diagnostic and export transitions |
| [Config coordinator](config-coordinator.md) | App intents, drafts, Save and operation reconciliation |
| [Observation coordinator](observation-coordinator.md) | Current/retained evidence, source joins and invalidation |
| [Diagnostics](diagnostics.md) | Core-owned autonomous runs, fixtures, assessment and real remaining scope |
| [Storage](storage.md), [persistent state](state.md) | Files, wire policy, resources and lifetime |
| [Protocol](protocol.md), [limits](limits.md) | Shared control/telemetry and authoritative capacity contracts |
| [Detection vectors](detection-vectors.md), [threat model](threat-model.md) | Coverage, known gaps and evidence limits |
| [Debug bundles](debug-bundle.md), [LSPosed debugging](lsposed-hook-debugging.md) | Retained read models, exports and hook investigation |
| [Development](development.md), [ADB root debugging](adb-root-debugging.md) | Build/validation and controlled device investigation |
| [Kernel test configuration](ci-kernel-config-trim.md) | Reference-kernel trim and independent ABI guards |
| [Migration](migration.md) | Isolated legacy readers and source preservation |
| [Changelog](changelog.md), [releasing](releasing.md) | Release tooling; running a release requires separate authorization |

## Decisions and historical evidence

[`rfcs/`](rfcs/README.md) records proposals and explicit decisions. Accepted means
approved implementation, not a completed feature or device proof. Reopened means
renewed review; consult the latest scope before changing policy.

[`archive/`](archive/README.md) preserves dated audits, old plans, device evidence
and implementation history. Old VPN-dependent state machines and activator
instructions there are historical. Do not promote their checkboxes or test counts
into current status. `notes/` retains compatibility links for older work indexes.
