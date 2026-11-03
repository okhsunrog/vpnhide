# VPN Hide v2 roadmap

This is the single current work index. **Implemented code, incomplete code and
external acceptance are separate.** A model, document, compile check or partial
fixture does not complete a runtime feature. Main is an ongoing beta refactor;
this is not a release announcement.

Current user instructions: [offline guide](docs/help/README.md). Current contracts:
[documentation index](docs/README.md). Design decisions: [RFCs](docs/rfcs/README.md).
Historical evidence: [archive](docs/archive/README.md).

## Decisions and current priorities

- Keep the app picker drafts and **Save/Cancel** footer. Immediate capacity
  prevention uses shared Rust/Core limits and UID accounting; Core checks commits.
  Per-toggle optimistic autosave was cancelled.
- One user-facing full Core ZIP contains the native alternatives. Separate
  backend producer archives are internal build inputs. Core is the sole runtime
  policy/effect owner; the APK runs no root process.
- **Zygisk lifecycle policy is reopened.** ZIP deduplication remains required.
  The package-only passive-pair cutover has landed; no managed projection or new
  boot-bound activation policy has been implemented. Do not ship this as a
  completed Zygisk runtime. The consolidated [RFC0005](docs/rfcs/0005-unified-backend-lifecycle-policy.md)
  is Proposed and awaits review; [RFC0003](docs/rfcs/0003-conditional-zygisk-publication.md)
  remains reopened.
- Continue independent diagnostics, maintenance, Kotlin ownership, provisioning
  and verification work while that lifecycle review proceeds. Do not implement
  the unapproved backend-hint/sticky/prestage/Auto/Manual policy proposals.
  **Never reboot the phone automatically**: preparation and a user-chosen reboot
  are separate operations; no reboot timer.
- No release, device install/reboot, provider settings change, new fork repository
  or production secret retrieval is authorized by this work index.

## Current implementation cycle

Current paired APK/Core RPC26; private journal12 reads schemas1–11. Kernel
control2/telemetry1 is unchanged; these numbers alone require no builtin rebuild.

| Feature | Actual status | Next concrete work / acceptance |
| --- | --- | --- |
| Core writer, journal, typed commands and persistent client | Implemented foundation | Preserve holds, attempt budgets, operation identity and migration readers. [Architecture](docs/core-architecture.md), [IPC](docs/core-ipc-auth.md). |
| Picker capacity and draft UX | Implemented | Preserve Save/Cancel and local shared-policy admission; no Kotlin numeric duplicate or save-only rejection. [Limits](docs/limits.md), [config coordinator](docs/config-coordinator.md). |
| Backend selection, authorization and bounded fallback | Partial | Verified catalog, frozen preparation, loader FD/image proof, revocation and exposure fences exist. Mixed-ABI genuine 32-bit bootstrap and complete production lifecycle remain. Publication changes are paused under RFC0003. |
| Single Core distribution and ZIP dedup | Package slice implemented; runtime incomplete | Descriptor/assembly and replacement checks enforce one passive ABI pair and no main-Core autoload directory. Preserve old-source image fencing during migration. Resolve lifecycle/placement before completing Zygisk runtime; no new projection executor exists. |
| Automatic GKI/KMI selection | Partial, approved for implementation | Manifest-derived candidates, ELF capabilities and separate experiment budgets exist. Implement real Core preflight dispatch/reconciliation and typed resolution; All seven pinned DDK engineering builds now retain complete real loader CRCs; shared assembly/runtime admission rejects incomplete GKI. [Qualification evidence](docs/archive/2026-11-03-gki-modpost-qualification.md). A clean same-commit full bundle also passed at `12c78631`; actual kernel/device evidence remains and production experimental admission stays off. [RFC0001](docs/rfcs/0001-automatic-gki-kmi-selection.md). |
| Autonomous diagnostics | Partial, approved for implementation | Core run/cache/assessment, genuine Android worker, interface/binding/address and several framework controls exist. Complete routes/rules, remaining proc/sys and callback/PendingIntent scenarios, narrower fingerprints and policy health. Address proof requires both syscall actors, both families and independent NULL-buffer controls; unavailable stimuli remain unverified. KPM NULL-buffer length filtering remains a code task. Required plans must not shrink to observed successes. [RFC0002](docs/rfcs/0002-autonomous-diagnostics.md), [current scope](docs/diagnostics.md), [address qualification](docs/archive/2026-11-03-native-address-fixture-qualification.md). |
| VPN-independent Dashboard | Implemented removal | Foreign-VPN state, APK split-tunnel gates/listeners and old prompts/domain owners removed. Preserve this independence in startup, diagnostics and all localized help. Real hook/fixture Connectivity observations remain. |
| Controlled proc interface observations | Implemented observation/evaluation slice; kernel concealment gap | Both libc/direct-syscall readers cover `/proc/net/dev` and `/proc/net/if_inet6` with owned positive/negative and cleanup controls. Current `.ko` QEMU execution exposes all four leaks; implement/qualify kernel filtering separately. Do not shrink the required plan or infer concealment from Android permission denial. This does not qualify proc routes, sockets or sysfs/proc-sys. [Evidence and scope](docs/archive/2026-11-03-proc-interface-fixture-qualification.md). |
| Feature-granular health and fallback policy | Unresolved review | Separate load, config delivery, required hook coverage and behavioral proof. Resident `status.error != 0` currently erases the admission mask while KPM configuration can accept PartialHooks. Do not fix this by silently loosening admission or adding an unapproved user switch. Diagnostic failure is not an automatic fallback trigger. |
| Unified native read models and exports | Partial | Selection, delivery, runtime and coverage are distinct Core facts and shared export inputs. Complete lightweight policy-health/process-generation attribution and remaining orchestration transfers. [Debug bundles](docs/debug-bundle.md). |
| Core repair, replacement, update and bootstrap | Partial foundation | Authenticated exact package preparation, private staging and executable/image fencing exist. Implement common durable executor, accepted sequence floor outside receipt pruning, actual manager operations, staged readback, post-reboot verification and APK no-su bootstrap/handoff. No silent Core flash. [Maintenance](docs/core-maintenance.md). |
| Separate KPatch provisioning | Partial offline model | Runtime/tool identity, kernel qualification and boot consent/write receipts exist. Concrete extract/patch/repack and safe device adapters remain. Fork publication target is unresolved; no arbitrary repository or real boot write. [Provisioning code](crates/provisioning/src/lib.rs). |
| Kotlin → Rust ownership cleanup | Partial | Import/export codec, self-target preview, canonical edit reducer/diff/write paths, validation, role-set/picker Save transforms and manual/auto-hidden reconciliation now share Rust owners through local FFI; duplicate Kotlin implementations are removed. Domain tests moved with their owners, with imported TCP/UDP rules covered through coordinator preparation and real Core persistence. Save/Cancel, same-value intent and uncertain-operation recovery remain. Core boot self-target preparation and typed APK Retry now use the durable Core writer. Move remaining Android-observation reconciliation scheduling, capture/export and agent domain orchestration; retire their replaced owners/callers/tests. Keep Android observations, permissions, SAF/share, UI drafts and localization as adapters. No RPC on hook hot paths. [Historical audit](docs/archive/2026-11-02-kotlin-core-architecture-audit.md). |
| Host tooling boundaries and approved 13-file port | Replacement cutover implemented | Shared Rust ELF/bundle/signing/distribution/update-feed/reset/QEMU owners and callers replaced the approved Python/Shell logic. Real bionic probes and minimal bootstrap remain. Destructive cold rescue is still unimplemented; no offline journal wipe. [Tooling record](docs/archive/2026-11-02-host-tooling-audit.md). |
| Core attention and Android notifications | Approved design; not implemented | Core active attention/update scheduling and durable dedup; explicit private wake → bounded RPC → native channels. Respect denial, force-stop and boot/unlock grace. Missing-Core bootstrap is separate. [RFC0004](docs/rfcs/0004-core-attention-notifications.md). |
| Documentation ownership | Current/archive cutover implemented | Current localized help/contracts, decision metadata and dated evidence have separate homes. Maintain source-linked statuses, localized shipped instructions and link/anchor checks with each change. |
| Rust 1.99.1 migration | Implemented | Keep strict lint checks and both Android ABIs; no global Clippy relaxation. Core/FFI/Zygisk now share explicit Android 16 KiB linking; six actual ABI images passed LOAD alignment/congruence checks. Real device acceptance remains separate. |

## Zygisk lifecycle review boundary

[Consolidated RFC0005](docs/rfcs/0005-unified-backend-lifecycle-policy.md) is now
published in English for review: Auto/Manual, feature-granular partial health,
finite recovery, provider/discovery boundaries and the migration/update matrix.
Publication is not implementation approval. It proposes superseding RFC0003's
strict default; current code and safety guards remain unchanged.

[The preserved proposal](docs/rfcs/0003-conditional-zygisk-publication.md) is reopened,
not authority to continue a new activation policy. Review fresh install,1.x→2.x,
compatible APK client/UI updates, Core patch/minor updates, kernel changes, builtin
appearance, fallback and boot failure with ordinary multi-week uptime.

Remembering a successful backend, keeping an earlier preload, combining staging
with one already-needed boot, and accepting inert cached mappings are proposals.
Mapping is not hook exposure. Existing same-boot possible-hook-consumption fences
remain mandatory; removing authorization does not unload hooks. Compatible APK
client/UI changes must not automatically imply reinstalling Core. Java-hook or
runtime-image changes have their own lifecycle requirements. No blanket reboot
count or safe coexistence is asserted before review.

Auto by default plus an exact Manual backend with no hidden fallback is a new
product proposal, not an implemented setting or final permission to resume policy
work. Partial useful coverage, total failure and uncertain load require separate
decisions. Preserve current no-effect-only bounded fallback and exposure guards
until that review concludes.

Zygisk is now explicitly **excluded from Auto, automatic fallback and standby**,
recorded in RFC0005's 3 November amendment. Only a current informed “Use Zygisk”
backend choice permits discovery preparation and grants; prior consent/use,
provider presence and imports are not an Auto license. Returning to Auto never
permits silent reentry. Sufficient working kernel protection produces no offer;
a justified last-resort offer is deduplicated and declined offers do not nag.
Advanced settings allow reconsideration/revocation with the injection/detection
warning. Existing cached images/hooks need separate retirement. This is a
confirmed product requirement, **not implemented code** or permission to lift the
broader policy pause. No automatic reboot, unsafe live Kmod↔KPM handoff, or silent
KPM provisioning follows from kernel-only Auto.

## Final build and acceptance gates

| Gate | What is established | What remains |
| --- | --- | --- |
| Host/kernel test evidence | Rust/JVM/FFI and strict host/ARM checks are recorded with code slices. Typed QEMU passed full ko39 and KPM35 scoped vectors, RAW/protected fixture and preflight on a reference6.1 kernel. | This is not the full fresh production seven-KMI matrix or Android proof. KPM NULL SIOCGIFCONF size query remains an explicitly separate gap. |
| Zygisk safe-copy host suite | New retirement regression and both Android ABI Clippy pass. | This cloud host returns ENOSYS for self process_vm_readv; two existing host tests fail there. Do not skip or weaken pointer safety to report success. |
| Final artifacts | Complete bundle assembly verifies hashes, size, ELF and provenance; public asset allowlist is APK/full Core/descriptor. | Clean `12c78631` ARM32/ARM64 Core and Zygisk, KPM and all seven CRC-qualified KMI payloads passed assembly: 20 entries, 10 payloads, one passive Zygisk pair; ZIP 4,306,807 bytes, SHA256 `39f93785b531085053290db5f3769b000df2c67c61ae9d58559494d43286e04e`. This is a source checkpoint; rebuild again for later final code. Earlier CRC-free bundle remains quarantined. A daemon base ZIP is not that bundle. Production-signed APK needs the authorized CI key; no temporary key substitute. |
| Startup regression | APK coalescing and bounded Core root-snapshot admission/cancellation implemented; current cloud regressions pass. | **At the very end**, rerun cold readiness, competing probe admission, reconnect and Retry for `core root snapshot probe exit=-1`; no false-green masking. The current slice passed 622 selected Rust tests and 42 JVM startup/root/session regressions, including real-child cancellation and late-dispatch controls. Final phone check still needed. |
| Device acceptance | Old Pixel/builtin acceptance is historical evidence only. | Later install compatible paired artifacts when authorized; verify startup, canonical preservation, selected backend, real Android/Zygisk/SELinux and framework fixture behavior after the required lifecycle boundary. |
| Irreversible/external work | Offline code and safe tests are authorized. | Real boot writes/reboots/provider changes need separate device consent; fork target and production key access remain distinct blockers. |

## Retained backlog — no automatic implementation approval

All original ideas remain in the [archived roadmap](docs/archive/2026-11-02-ROADMAP.md).
These entries preserve scope, not old claims of completion or current product policy.

| Direction | Current disposition |
| --- | --- |
| VPN-preserving concealment and explicit Network binding | Proposed; investigate routing preservation separately from hiding or Dashboard readiness. |
| Network-handle edge cases and callback lifecycle across Android versions | Continue controlled fixture coverage under RFC0002; broader product modes need their own decision. |
| Single-lookup route concealment | Deferred; separate from already implemented route dump hooks. |
| 32-bit compat SIOCGIFCONF and KPM NULL size-query | Known native coverage gaps, not covered by compat socket-bind success. |
| Permissive SELinux severity/coverage | Revisit against measured applicability; no inference that an empty denied API proves hiding. |
| Additional interception/installation observability | Existing target counters are implemented; preserve their source attribution. Further coverage is separate work. |
| Non-target observer statistics | Proposed opt-in extension; not blanket hot-path logging. |
| Zygisk per-app counters | Intentionally unsupported by the previous design. No new cross-app writable channel or backend retirement decision follows from the historical note. |
| Custom VPN-interface prefixes | Proposed; generated defaults remain the implemented policy. |
| Split-tunnel and remote IP/DNS/latency limits | Keep accurate user guidance. External VPN status must not become a diagnostic prerequisite again. |
| Setup wizard and release-candidate channels | Historical proposals, not implemented or newly approved. Preserve in [archive](docs/archive/README.md); do not run release tooling or create tags. |

Update this index with the relevant implementation change. Link detailed contracts,
decisions and evidence rather than appending a second chronological work ledger.
