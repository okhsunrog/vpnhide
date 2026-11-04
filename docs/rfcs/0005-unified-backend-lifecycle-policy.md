# RFC 0005: Backend continuity and bounded KMOD qualification

Status: **Accepted for implementation and publication of verified code to main.**
Publication approval: 4 November 2026. Remaining code and device gates stay explicit.
Decision: 3 November 2026. This document replaces the earlier consolidated
proposal; its [original body is archived](../archive/2026-11-03-rfc0005-initial-proposal.md).
Acceptance authorizes code and cloud checks, not device changes, boot writes,
provider settings, releases or automatic reboot. Implementation and device proof
are separate entries in [ROADMAP](../../ROADMAP.md).

## Decision

Keep the user's actually working backend. If there is no working backend, prefer a
compatible Builtin implementation, then resolve one compatible bundled KMOD.
**There is no automatic fallback between backend families. Auto never newly selects
Zygisk.** A failed load, partial coverage, a diagnostic infrastructure problem or an
unavailable Java/Ports observation does not authorize trying another family.

An existing Zygisk user keeps that working backend after an update. On the first
APK launch after the update, offer a voluntary move if a compatible kernel backend
is available. A new Zygisk choice requires explicit selection and an accurate
warning that process injection can be detected, including by games and banking
apps. It is not an automatic rescue backend or standby option.

Core owns selection, compatibility evidence, desired versus effective state,
operation recovery and the offer's lifecycle. Kotlin renders typed read models and
submits user intent. Preserve picker drafts, Save/Cancel, immediate shared-policy
capacity prevention and uncertain-Save recovery.

## Selection table

| Situation | Core action |
| --- | --- |
| Working managed Builtin, KMOD or KPM | Retain it; changing package/catalog priority is not a reason to replace it. |
| Working existing Zygisk | Retain it, distinguish actual coverage from mapping, and consider the post-update migration offer. |
| No working backend, compatible Builtin present | Select Builtin. APK/Core RPC numbers alone do not require rebuilding a compatible builtin kernel. |
| No working backend or applicable Builtin; one sufficiently qualified KMOD | Freeze that final artifact and policy, then perform its single journaled real-load attempt. |
| KMOD candidates remain ambiguous | Qualify bounded preflight where permitted; retain ambiguity if evidence still cannot select one. |
| Selected family fails, is partial, or has uncertain effects | Report that state. Do not try another family automatically. |
| New explicit KPM choice | Require actual compatible authenticated runtime; provisioning/boot patching remains separately consented work. |
| New explicit Zygisk choice | Require informed selection, verified payload/provider applicability and normal authorization/exposure fences. |
| Explicit request conflicts with live hooks or a hold | Keep the effective state; publish refusal or a prepared user action/restart requirement. |

Fresh Auto does not silently provision a kernel runtime or invoke an unapproved
extra selection tier when Builtin/KMOD selection cannot finish. Explicit supported
alternatives remain advanced choices. A manual KMOD family choice still delegates
KMI/artifact resolution to Core; it does not expose manual GKI ZIP selection.
The product needs no separate mode switch in addition to the advanced Backend
choice. Normal users see protection status and a concrete necessary action.

## What counts as working

Observed residency, successful load, canonical persistence, confirmed policy
delivery, required feature coverage and behavioral proof are separate facts.
A passive mapping or path on disk is not proof of running hooks. Remembered
last-successful state is a validated continuity hint, not current authorization.

Retain an actually active implementation whose ownership and policy delivery are
established. Do not replace it because a different implementation now has a higher
static priority. Partial installed masks remain visible instead of being erased
by a nonzero status code. Missing required features and unverified diagnostics
remain explicit; neither produces a false full PASS or an automatic switch.
Unknown load/ownership/configuration effects retain their holds and prohibit a
second load. Lost replies are reconciled using the same operation identity.

A kernel change can invalidate remembered compatibility. Fresh selection after
such a change follows actual current facts, not an unconditional old-family pin.
Legacy running code may require an isolated adoption/migration path. Missing
modern generation evidence must not turn old mapping into authenticated new
coverage, or justify a competing backend in the same boot.

## Passive KMOD evidence

Resolve from the verified unified bundle and current device facts:

1. Full `uname` release and machine, plus Linux LTS. Distinguish kernel architecture
   from the Core process ABI; never infer kernel ABI from the APK library alone.
2. `.modinfo` vermagic from several standard modules corresponding to the running
   kernel. A KernelSU Next module is useful when its identity is bound to that
   kernel; its product name or an arbitrary file is insufficient.
3. Evidence tying files to current partitions/runtime: bounded allowlisted
   inventory, real ELF bytes/hash, module name, available build ID/srcversion and
   observed loaded-module identity. Record absent or conflicting bindings.
4. Manifest KMI, ELF machine, imports/version CRCs, provenance and other existing
   applicability checks for each immutable final bundled candidate.

Use independent modules and retain conflicts. Do not count repeated copies of one
file as corroboration, trust the first directory hit, or accept a foreign/OTA file
because its filename resembles a current module. Vermagic is evidence for
selection; it does not prove private structure ABI or all required behavior.
Unknown is distinct from compatible. `uname`'s Android tag is useful evidence,
not an unconditional override of contradictory current-module evidence.

**Never modify vermagic, version CRCs or candidate bytes to bypass loader checks.**
The payload tested, frozen in Prepared and really loaded must have the same hash.
The package remains one Core ZIP with manifest-derived alternatives and no manual
backend ZIP requirement.

## Bounded final-file preflight

Preflight can resolve remaining KMOD ambiguity only when the final files have a
hash-bound supported protocol/capability established before the syscall. An
unknown module parameter may be ignored by an older module and execute full init;
therefore merely supplying `preflight_only` is never admission.

- Use the actual final manifest-verified file/FD, not a special easy-to-load probe
  or a patched temporary binary. Validate real loader import/version records.
- Keep finite per-boot/context candidate budgets separate from the one real-load
  budget. Persist intent before entering the kernel and reconcile crashes without
  resetting attempts, holds or floors on upgrade.
- The exact qualified sentinel plus no module/proc/hook effects establishes the
  scoped loader preflight result. Negative errno alone is not full private-ABI
  proof; an unknown/interrupted result is not a safe failed attempt.
- A failed real load does not start a loop over other KMIs or other backend families.
  If two final candidates pass but cannot be distinguished, retain typed ambiguity
  instead of selecting the first one.
- Existing boot/exposure/ownership fences still apply. A hold cannot be cleared by
  deleting a journal or by treating preflight as an ordinary insmod trial loop.

Cloud qualification is engineering evidence for the exact tested Image/files.
Unknown vendor/private ABI and required Android/kernel acceptance remain honest
gates; they are not a reason to postpone independent dispatcher/recovery code.

## Zygisk continuity and explicit selection

Auto never introduces Zygisk, even after a historical consent. Provider presence,
passive payload availability and an imported preference are not a new selection.
Returning to Auto never permits silent later Zygisk reentry.

Verified existing Zygisk operation has an explicit continuity exception: retain
what already protects that user. This supersedes the old proposal's blanket
statement that previous use could not support continuation. It is not permission
to reuse an old boot's authorization, infer full ABI/process coverage or start
Zygisk on a fresh device. Preserve generation/recipient/image identity and truthful
legacy coverage boundaries while migrating ownership to Core.

For a new choice, show the injection/detection warning before discovery publication
and authorization. Kernel protection is preferred; do not claim that a kernel
backend guarantees undetectability or that a reported bank ban proves causality.
No standby publication, silent fallback, provider-wide rearm/reenable, unsafe
refresh or interference with other modules is authorized.

The Core ZIP retains one passive Zygisk ABI pair. A verified installed discovery
copy is permitted and remains Core-managed, with no independent activator, policy,
user ZIP or update channel. Main Core has no autoload `zygisk/`, including when the
provider is globally disabled. Provider cache/boot ordering and real generation
proof must be respected; exit zero does not certify a hot refresh.

## Post-update offer

For an existing Zygisk user, the first eligible APK launch after an update reads
Core's offer state. Offer voluntary migration only when Core has a compatible
kernel candidate. No usable candidate means no speculative prompt.

Track delivery/dismissal for that upgrade so reopening screens or reconnecting
does not repeat it. Decline keeps Zygisk. Accept submits a typed target intent;
it does not bypass hooks, compatibility, ownership, maintenance consent or a
required reboot. Existing hooked processes remain a separate lifecycle boundary.
The offer is not an automatic transition or a diagnostic-failure popup.

## Transition and maintenance safety

- Never reboot the phone automatically. Preparation and a user-chosen reboot are
  separate; there is no reboot timer.
- After possible Zygisk hook consumption, revocation does not unload process hooks
  and cannot authorize a kernel transition in that boot.
- A KMOD has no module_exit; unknown loading cannot be followed by a blind second
  load. Do not create automatic KMOD/KPM reboot chains.
- Passive library mapping alone is distinct from consumed authorization. Do not
  demand a cleanup-only reboot if kernel selection is otherwise safe and no hooks
  could have been installed; do not assume that absence without evidence.
- Keep the effective working backend during compatible APK/Core updates and long
  uptime. Prepare verified replacement before manager operations; do not remove
  the old working installation before the new one is ready.
- Compatible APK-only updates need no device reboot. A normal patch/minor update
  does not acquire a rule requiring two boots. Unknown load or real kernel changes
  can require a new boot, always chosen by the user.
- Preserve canonical config, pending intents, authorization history, attempts and
  journal migration readers. No offline journal wipe or downgrade through removal.
- Boot-image patch/write, runtime provisioning, backup/slot/OTA/AVB/readback checks
  and actual-device confirmation remain a separate component/operation.

Diagnostics, Java/Ports failures and foreign VPN state never become hidden backend
selection inputs. Autonomous diagnostics still derive required checks from the
supported contract and enabled features, not the successfully installed mask.
Fresh-worker PASS does not prove old game-process generation coverage; a genuine
secondary32-bit launcher is still distinct from ARMv7 compilation.

## Implementation and verification

Implement in coherent local commits, update Rust/Kotlin/FFI/migrations together
when a reachable contract changes, and retire replaced selectors/fallback owners
and obsolete expectations. Do not transplant Kotlin navigation, Android permission,
SAF/share or localization into root. Keep host tools on portable libraries.

Executable acceptance includes:

- Retaining each working family, including partial/legacy Zygisk without inventing
  coverage; no static-priority replacement or foreign-VPN diagnostic gate.
- Fresh Builtin/KMOD preference, conflicting/absent module witnesses, ELF/CRC/hash
  checks, finite qualified preflight and ambiguous two-pass refusal.
- No second family after load/config/health/diagnostic failure or unknown effects.
- Explicit new Zygisk warning/choice, no standby, post-upgrade offer deduplication,
  decline/accept recovery, no hidden Auto reentry and no automatic reboot.
- Crash/reconnect/upgrade/downgrade holds, immutable Prepared inputs, manager
  replacement ordering and boot/process exposure fences.
- Both compiled ARM ABIs, actual frame/ELF bounds, appropriate host/QEMU behavior,
  and separately listed device/provider/kernel/signing acceptance.

Main publication remains withheld until the maintainer separately confirms it.
No test count, helper or updated RFC is a claim that the production vertical or
physical-device acceptance is complete.
