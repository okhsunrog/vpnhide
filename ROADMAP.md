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
- **Backend continuity policy is accepted for implementation.** Keep the
  actually working backend; fresh selection prefers compatible Builtin then KMOD.
  No inter-family automatic fallback. Existing Zygisk continues, with a voluntary
  compatible-kernel offer after upgrade; new Zygisk needs explicit selection and
  warning. [RFC0005](docs/rfcs/0005-unified-backend-lifecycle-policy.md) supersedes
  the earlier recovery proposal;0003 packaging and safety constraints remain.
- **Verified policy changes may be published to main** (maintainer confirmation
  received 4 November 2026). Code, compile checks and device acceptance remain separate. Never reboot the phone automatically.
- Independent diagnostics, maintenance, Kotlin ownership, provisioning and
  verification retain their prior authorization. The genuine secondary32-bit
  Android launcher remains deferred.
- On 5 November the maintainer authorized the scoped install-time handoff and installation/testing on the connected phone, preserving user data and recovery. The verified Pixel 8 Pro currently uses Builtin; this cannot establish existing-Zygisk device continuity.
- No release, provider settings change, new fork repository
  or production secret retrieval is authorized by this work index.

## Current implementation cycle

Current paired APK/Core RPC38; private journal21 reads schemas1–20. Kernel
control2/telemetry1 is unchanged; these numbers alone require no builtin rebuild.

| Feature | Actual status | Next concrete work / acceptance |
| --- | --- | --- |
| Core writer, journal, typed commands and persistent client | Implemented foundation | Preserve holds, attempt budgets, operation identity and migration readers. Scoped instance/floor/staging locks now end with their owning process scope despite inherited pre-exec FDs; live contenders remain excluded. [Architecture](docs/core-architecture.md), [IPC](docs/core-ipc-auth.md), [fork regressions](docs/archive/2026-11-03-maintenance-lock-lifetime.md). |
| Picker capacity and draft UX | Implemented | Preserve Save/Cancel and local shared-policy admission; no Kotlin numeric duplicate or save-only rejection. [Limits](docs/limits.md), [config coordinator](docs/config-coordinator.md). |
| Backend continuity, selection and authorization | Managed lifecycle connected; final qualification pending | Explicit warned managed Zygisk publication and boot-bound withdrawal now use the existing single command writer, with atomic receipts, immutable Unknown and no manager replay. New-boot exact tombstone/no-grant proof is rechecked around kernel admission; current-boot active code and current journal-owned policy are retained. Authenticated Core updates stage both Core and the managed companion before any retirement; source-only updates after withdrawal retain the inert tombstone. The voluntary compatible-kernel upgrade offer has Core-owned eligibility and durable display/keep receipts, typed RPC34/FFI and a thin APK adapter. Earlier pre-handoff baseline checks passed; they are not final qualification of the current installer changes. The focused installer/boot handoff run passed 23 tests. Final Core ARM binaries/full same-commit bundle and real provider/device acceptance remain separate. Subsequent non-Zygisk choices and informed reentry after confirmed new-boot withdrawal retain separate receipts and bounded prior-cycle history. Explicit Repair can settle current-boot bootstrap Unknown only after exact grant/source readback and independent fresh specialization; the original Unknown and exposure remain. Explicit withdrawal can also settle a missing or exact previous bootstrap grant without republishing Zygisk; foreign grants and independent holds remain refused. Explicit warned adoption of the recognized old separate Zygisk module now uses the production writer, verified new ABI bytes and durable retirement intent. Bounded old-Core autoload continuity now uses immutable installer capture, atomic grant-free companion publication and new-boot adoption on the existing writer; unproven standalone legacy use still needs explicit warned adoption. Reauthorizing Zygisk without a current exact grant remains held. Secondary32 and the proposed system-server adapter are deferred to the separate published research branch by the maintainer; they do not block current delivery and no hook is present in this checkout. The ordinary Core replacement command still refuses incidental legacy retirement; the manager install hook owns the bounded old-Core handoff. Fresh Auto remains Builtin then KMOD, without inter-family fallback. |
| Single Core distribution and ZIP dedup | Package and managed runtime paths connected; qualification pending | One passive Zygisk ABI pair remains in the full Core ZIP, with no Core autoload directory. Informed managed publication, boot-bound withdrawal and authenticated combined update now have production writer callsites. Staging preserves this boot's exact running code/grant and never certifies hooks exited. Bounded old-Core continuity is connected; unproven standalone legacy continuity and the final same-commit full bundle remain separate. |
| Automatic GKI/KMI selection | Partial, approved for implementation | Manifest-derived candidates, ELF capabilities and separate experiment budgets exist. Final-byte preparation and real journaled preflight dispatch are implemented for the isolated Core QEMU oracle; production selection dispatch/reconciliation and typed resolution remain code work. All seven pinned DDK engineering builds retain complete real loader CRCs; shared assembly/runtime admission rejects incomplete GKI. [Qualification evidence](docs/archive/2026-11-03-gki-modpost-qualification.md). A clean same-commit full bundle also passed at `12c78631`; actual kernel/device evidence remains and production experimental admission stays off. [RFC0001](docs/rfcs/0001-automatic-gki-kmi-selection.md). |
| Genuine secondary32-bit Android worker | Deferred by maintainer; outside current delivery | Ordinary `:diagnostics` uses the selected package ABI. On 4 November 2026 the maintainer deferred the proposed system-server hook/launcher to [branch `research/secondary32-abi`](https://github.com/okhsunrog/vpnhide/tree/research/secondary32-abi) (archive-only commit `08fc73b2`); it is not a release blocker. A later safe real Zygote32 path on mixed-ABI devices still needs actual ABI loading/runtime qualification. ARMv7 compile/ELF and ARM64 worker evidence do not satisfy it; no borrowed image proof or forced APK restart. [Current scope](docs/diagnostics.md#worker-abi-coverage), [rejected instrumentation candidate](docs/archive/2026-11-03-diagnostic-worker-abi-launch.md). |
| Autonomous diagnostics | Partial, approved for implementation | Core run/cache/assessment, genuine Android worker, interface/binding/address and several framework controls exist. Core and the Android worker now run the [owned UP route fixture](docs/diagnostics.md#implemented-scope-and-remaining-implementation) for scoped netlink/proc routes and policy rules, with cleanup admission and typed retained results. Pixel 8 Pro on Android 17 and Builtin confirmed all eight netlink route and rule vectors. Native flags, MTU, getifaddrs, ioctl addresses and socket binding also qualified after decoupling the worker controls and stabilizing the fixture population. Pixel access tests across six target SDKs left RTM_GETLINK and the proc groups unverified. The scoped default-network check passed two Pixel runs during Direct Boot after the RPC37 update, bringing the current Pixel result to 13 confirmed groups out of 20. Callback probes for LISTEN, default and REQUEST qualified on Pixel in 29 of 32 repeated runs and brought the current result to 14 confirmed groups out of 20. PendingIntent, Package Visibility and Ports behavioral fixtures, narrower fingerprints and policy health remain implementation work. Framework RAW grants exist. The `.ko`/builtin RAW provider, Root controller, genuine Android single-thread handshake, namespace/birth revalidation and paired assessment are implemented with cloud coverage. Native observations reach Core before Java collection and survive later worker failure. Run authority and active waits now use suspend-aware boot deadlines; actual Android suspend/cleanup acceptance remains separate. KPM/Zygisk RAW adapters remain code work. The [KPM runtime lifecycle review](docs/archive/2026-11-03-kpm-raw-lifecycle-review.md) found an unresolved control/unload boundary; its disabled offline prototype is not integrated or production proof. Android and actual builtin acceptance remain separate. Contract11 withholds native proof without app RAW; filesystem applicability still follows the desired feature. Address proof requires both syscall actors, both families and independent NULL-buffer controls; unavailable stimuli remain unverified. KPM native ARM64 NULL-buffer filtering is implemented and reference-qualified; genuine compat-process coverage remains separate. Six optional sysfs/proc-sys directory vectors now have libc/direct-getdents64 observations, same-worker RAW qualification and independent cleanup controls; [qualification and scope](docs/archive/2026-11-03-native-directory-fixture-qualification.md). Eighteen optional stat/access/open path checks now retain independent positive RAW, ordinary-object and cleanup controls; [qualification and limitations](docs/archive/2026-11-03-native-filesystem-fixture-qualification.md). Genuine secondary32-bit Android worker launch remains an open implementation gap; ARMv7 compilation is not runtime coverage. Required plans must not shrink to observed successes. [RFC0002](docs/rfcs/0002-autonomous-diagnostics.md), [current scope](docs/diagnostics.md), [address qualification](docs/archive/2026-11-03-native-address-fixture-qualification.md). |
| VPN-independent Dashboard | Implemented removal | Foreign-VPN state, APK split-tunnel gates/listeners and old prompts/domain owners removed. The unused Rust uncontrolled Pass/Fail suite, observation-v1 codec and `probe checks` CLI are also retired; typed collectors and Core evaluation remain. The unused gate-era synchronous Kotlin network collector is also removed; agent descriptions and capture comments no longer claim a routing self-test. Preserve this independence in startup, diagnostics and all localized help. Real hook/fixture Connectivity observations remain. |
| PackageManager package/UID observation codec | Implemented ownership slice | Portable Rust owns package rows, UID union and per-user scan markers for Core and APK FFI. Profile display metadata and Android backstop acquisition remain adapters; fresh Core inventory still admits mutations. [Contract](docs/observation-coordinator.md#package-inventory-format-owner), [qualification](docs/archive/2026-11-03-package-inventory-codec.md). |
| Root/debug snapshot codec | Implemented ownership slice | Portable Rust owns framing, required sections and bounds; Core emits the shared format and typed local FFI supplies APK adapters. Old Kotlin parser and source guards are removed; real output, independent fixture and FFI/JVM regressions remain. Snapshot reads admit no suite. [Scope and verification](docs/archive/2026-11-03-shared-probe-frames.md). |
| Controlled proc interface observations | Implemented observation/evaluation slice; kernel concealment gap | Both libc/direct-syscall readers cover `/proc/net/dev` and `/proc/net/if_inet6` with owned positive/negative and cleanup controls. Current `.ko` QEMU execution exposes all four leaks; implement/qualify kernel filtering separately. Do not shrink the required plan or infer concealment from Android permission denial. This does not qualify proc routes, sockets or sysfs/proc-sys. [Evidence and scope](docs/archive/2026-11-03-proc-interface-fixture-qualification.md). |
| Feature-granular native health | Partial implementation | Shared protocol admission retains observed PartialHooks masks, requires complete requested coverage and refuses fatal/unknown status. Load, config delivery, required feature coverage and behavioral proof remain separate; diagnostic failure never initiates an inter-family switch. Complete typed feature-impact/policy-health attribution without claiming complete process/ABI coverage. |
| Unified native read models and exports | Partial | Selection, delivery, runtime and coverage are distinct Core facts and shared export inputs. APK telemetry readers now use the shared Rust wire grammar through local typed FFI; LSPosed emission stays independent of app native code. Attachment recommendations use one shared Rust policy over typed checks/boot evidence; Kotlin retains currentness joins, localization and sheet overrides. Unsigned counter delta/reset classification is shared Rust through a bounded local FFI batch for capture/agent and the same scalar owner for forensic rendering. Capture logging leases and recorder caps now share suspend-aware limits and retain an explicit clock-failure reason; [scope and evidence](docs/archive/2026-11-03-capture-boot-clock-qualification.md). Complete lightweight policy-health/process-generation attribution, remaining counter projections and capture/export orchestration transfers. [Debug bundles](docs/debug-bundle.md). |
| Core repair, replacement, update and bootstrap | Existing-Core and managed companion replacement connected; bootstrap incomplete | Explicit durable command now binds authenticated full-ZIP preparation, accepted floor, retained active work, bounded manager dispatch, actual staged/running image proof, guarded companion retirement and reboot verification. APK has a bounded explicit SAF adapter and typed progress; no silent install/reboot. Standard manual KernelSU installs can activate a compatible Builtin worker from modules_update after manager completion, with same-RPC supervisor readiness and in-boot recovery. The accepted [private readiness protocol](docs/rfcs/0005-unified-backend-lifecycle-policy.md#same-boot-worker-readiness--15-november-2026) remains code work. Authenticated managed replacement keeps its reboot proof. Managed companion updates consume authenticated incoming package bytes; an already withdrawn companion stays an inert tombstone across source updates. Unhandled legacy Zygisk installations are preserved by refusing replacement. Missing-Core/incompatible-RPC rootless manager handoff, automated artifact retrieval and richer refusals remain code work. Real manager/fork/SELinux/reboot and production signing/final bundle gates remain separate. [Maintenance](docs/core-maintenance.md). |
| Separate KPatch provisioning | Partial offline model | Runtime/tool identity, kernel qualification and boot consent/write receipts exist. Concrete extract/patch/repack and safe device adapters remain. Fork publication target is unresolved; no arbitrary repository or real boot write. [Provisioning code](crates/provisioning/src/lib.rs). |
| Kotlin → Rust ownership cleanup | Partial | Import/export codec, self-target preview, canonical edit reducer/diff/write paths, validation, Ports rules/presets/agent intents, role-set/picker Save transforms and manual/auto-hidden reconciliation now share Rust owners through local FFI; duplicate Kotlin implementations are removed. Domain tests moved with their owners, with imported TCP/UDP rules covered through coordinator preparation and real Core persistence. Save/Cancel, same-value intent and uncertain-operation recovery remain. Core boot self-target preparation and typed APK Retry now use the durable Core writer. Move remaining Android-observation reconciliation scheduling, capture/export and agent domain orchestration; retire their replaced owners/callers/tests. Keep Android observations, permissions, SAF/share, UI drafts and localization as adapters. No RPC on hook hot paths. [Historical audit](docs/archive/2026-11-02-kotlin-core-architecture-audit.md). |
| Host tooling boundaries and approved 13-file port | Replacement cutover implemented | Shared Rust ELF/bundle/signing/distribution/update-feed/reset/QEMU owners and callers replaced the approved Python/Shell logic. Real bionic probes and minimal bootstrap remain. Destructive cold rescue is still unimplemented; no offline journal wipe. [Tooling record](docs/archive/2026-11-02-host-tooling-audit.md). |
| Core attention and Android notifications | Approved design; not implemented | Core active attention/update scheduling and durable dedup; explicit private wake → bounded RPC → native channels. Respect denial, force-stop and boot/unlock grace. Missing-Core bootstrap is separate. [RFC0004](docs/rfcs/0004-core-attention-notifications.md). |
| Documentation ownership | Current/archive cutover implemented | Current localized help/contracts, decision metadata and dated evidence have separate homes. Maintain source-linked statuses, localized shipped instructions and link/anchor checks with each change. |
| Rust 1.99.1 migration | Implemented | Keep strict lint checks and both Android ABIs; no global Clippy relaxation. Core/FFI/Zygisk now share explicit Android 16 KiB linking; six actual ABI images passed LOAD alignment/congruence checks. Real device acceptance remains separate. |

## Backend policy implementation boundary

[RFC0005](docs/rfcs/0005-unified-backend-lifecycle-policy.md) is accepted for
implementation; verified changes may be published to main under the separate
4 November confirmation. The original
large proposal and RFC0003's previous defaults are historical, not competing
policy authority.

Fresh Auto chooses compatible Builtin then KMOD, with no automatic inter-family
fallback. The implementation removes the prepared successor queue and
ends execution after one durable attempt, including proven no-effect rejection.
Unknown effects, attempt budgets, possible Zygisk exposure and all ownership
fences remain. Advanced next-boot kernel choice and warned managed Zygisk staging use durable intent and preserve the current managed backend. Authenticated managed updates and voluntary upgrade offers now have writer callsites. Explicit warned separate-module adoption and scoped subsequent choice/reentry settlement after confirmed withdrawal now have writer callsites. Bounded old-Core autoload continuity now has installer and new-boot writer callsites. Final-file qualification and physical-device acceptance remain pending; arbitrary standalone legacy use is not inferred.

Passive module inventory and selection now share the common ELF metadata owner:
only live sysfs build-ID matches can corroborate vermagic, distinct names/hashes
are required to settle a tie, and conflicts survive incomplete scans. Bounds on
entries/bytes/records and a bounded same-image child exist. Valid prefixes survive
timeout/failure with incomplete status. RPC32/journal15 retain current observations
and current-boot conflict evidence separately, including refused preparation.
Incomplete later reads and receipt pruning cannot erase the conflict; the same
bounded typed evidence reaches generated FFI and forensic exports. Final-file
manifest bytes are frozen as well as the normalized digest. This is not
kernel/device ABI acceptance or permission to modify candidate bytes.

Partial resident readiness now shares the protocol owner used by KPM delivery;
preparation retains its actual installed mask and continues to reject missing
required coverage and fatal/unknown states. This does not complete the remaining
cross-upgrade continuity, informed Zygisk choice or voluntary Zygisk transition flow.

Existing Zygisk continues where actual working ownership can be established. It
never licenses new Auto selection, standby publication or silent reentry. Core
never automatically reboots, silently provisions KPM, bypasses ABI checks or
performs an unsafe live KMOD/KPM handoff. Provider cache and already-hooked
process retirement remain separate lifecycle boundaries.

## Final build and acceptance gates

| Gate | What is established | What remains |
| --- | --- | --- |
| Host/kernel test evidence | Rust/JVM/FFI and strict host/ARM checks are recorded with code slices. Typed QEMU passed ko39 scoped vectors plus RAW/protected fixture and preflight on reference6.1. KPM now requires native NULL-query size/fill agreement: one final payload passed 36 accepted rows on all seven GKI and five legacy reference Images; [exact proof and platform/N/A distinctions](docs/archive/2026-11-03-kpm-ifconf-qualification.md). | This is not the full fresh production seven-KMI bundle matrix or Android/32-bit compat proof. Unknown vendor layouts still require qualification. |
| Zygisk safe-copy host suite | New retirement regression and both Android ABI Clippy pass. | This cloud host returns ENOSYS for self process_vm_readv; two existing host tests fail there. Do not skip or weaken pointer safety to report success. |
| Final artifacts | Complete bundle assembly verifies hashes, size, ELF and provenance; public asset allowlist is APK/full Core/descriptor. | Clean `21cd31ee` ARM32/ARM64 Core and Zygisk, KPM and all seven CRC-qualified KMI payloads passed assembly: 20 entries, 10 payloads, one passive Zygisk pair; ZIP 5,335,293 bytes, SHA256 `e1413d0093faf03846a4fdfb0b594cafc2264a0e5154669501a5ac7200ed1fe0`. Its unsigned release APK passed R8; all twelve actual executable/shared ELF images passed 16 KiB alignment/congruence. [Exact checkpoint and limits](docs/archive/2026-11-03-clean-bundle-21cd31ee.md). Rebuild later final code. Earlier CRC-free bundle remains quarantined. A daemon base ZIP is not that bundle. Production-signed APK needs the authorized CI key; no temporary key substitute. |
| Startup regression | APK coalescing and bounded Core root-snapshot admission/cancellation implemented; current cloud regressions pass. | **At the very end**, rerun cold readiness, competing probe admission, reconnect and Retry for `core root snapshot probe exit=-1`; no false-green masking. The `21cd31ee` source checkpoint passed 591 selected Rust tests and 38 targeted JVM startup/root/session regressions, including real-child cancellation and late-dispatch controls. This is not final acceptance after the remaining code. Final phone check still needed. |
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
| 32-bit compat SIOCGIFCONF | Still a native coverage gap, not covered by native ARM64 NULL-query or compat socket-bind success. |
| Permissive SELinux severity/coverage | Revisit against measured applicability; no inference that an empty denied API proves hiding. |
| Additional interception/installation observability | Existing target counters are implemented; preserve their source attribution. Further coverage is separate work. |
| Non-target observer statistics | Proposed opt-in extension; not blanket hot-path logging. |
| Zygisk per-app counters | Intentionally unsupported by the previous design. No new cross-app writable channel or backend retirement decision follows from the historical note. |
| Custom VPN-interface prefixes | Proposed; generated defaults remain the implemented policy. |
| Split-tunnel and remote IP/DNS/latency limits | Keep accurate user guidance. External VPN status must not become a diagnostic prerequisite again. |
| Setup wizard and release-candidate channels | Historical proposals, not implemented or newly approved. Preserve in [archive](docs/archive/README.md); do not run release tooling or create tags. |

Update this index with the relevant implementation change. Link detailed contracts,
decisions and evidence rather than appending a second chronological work ledger.
