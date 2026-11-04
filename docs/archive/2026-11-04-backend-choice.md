# Device-local next-boot kernel choice — 4 November 2026

Implementation slice of [RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md),
not completion of its Zygisk lifecycle. Current contracts are RPC33 / journal16
(reads1–15); backend control2 / telemetry1 remain unchanged. No phone operation,
provider setting, release, production signing or automatic reboot occurred.

## Implemented

- Generated typed `StageBackendChoice`, private revisioned preference and desired
  versus confirmed-delivery read model. Portable canonical imports grant no choice
  or consent. FullReset clears the preference without erasing journal authority.
- Existing command writer, exact config and preference revisions, complete UID
  inventory, full verified catalog and final manifest/root identity checks.
  Preparation happens before writer admission; it performs no backend effects.
- Initial intent is durable before acceptance. New preference and terminal receipt
  commit in the same atomic journal document. A final sync failure freezes mutation
  and leaves admission uncertain; restart syncs/reconciles retained bytes, never
  replays a command. An initial Running-only intent proves no preference landed.
- No new family is selected until a different kernel boot identity. A daemon
  restart restores its actual same-boot decision. Exact family selection never
  substitutes another family. Permanent Builtin cannot be staged away under the
  same kernel. Legacy recipient recovery remains an isolated compatibility path.
- Kernel-only advanced chooser through the existing config coordinator, including
  the same operation-ID query recovery and draft protection. All three locales
  explain next-device-boot intent without promising immediate runtime changes.
- Shared local Rust picker decisions cover every distinct current/staged family,
  with accepted complete UID accounting. Core Save repeats both projections against
  one inventory and refuses before persistence. Proven reductions remain possible
  with missing inventory. Removed the Kotlin increase/fits classifier and obsolete
  standalone reduction FFI. Picker Save/Cancel/footer remain.

## Verification scope

Complete host Rust run: 627 tests passed, none failed/ignored, including actual
Zygisk crash recovery and startup/readiness/root-probe contention. An additional
real-RPC capacity regression passed with the two existing capacity tests: a valid
current-family wire exceeding the retained desired-family ceiling writes no config,
control bytes, revision or operation receipt. The staged-capacity and warning
admission regressions make 629 distinct tests across the complete and focused
runs. Android JVM tests (587, no failures
or skips), ktlint, detekt, CPD and Android lint also pass. Strict host all-target Clippy includes fixture-oracle;
production API28 ARM64 and ARMv7 libraries/binaries pass strict Clippy. Both actual
JNI images pass 16 KiB LOAD alignment and offset/address congruence.

The initial full run exposed a real regression: a catalog-only restore pin was
applied to a legacy Zygisk recipient, blocking its repair before the injected crash
boundary. Restricting that pin to catalog-bound transitions preserves the isolated
legacy recovery adapter; the original crash regression passes without relaxing
new explicit-choice admission or warning requirements.

Evidence is retained outside the source tree at
`/workspace/vpnhide-build-evidence/backend-choice/`. JNI bytes are from the RPC33
working tree based on bb6a2450, with explicit dirty-source provenance; they are not
claimed to be a clean same-commit full bundle or Android runtime acceptance.

## Still code, separately from device gates

- Actual managed Zygisk discovery projection, explicit warning/choice flow,
  cross-upgrade continuity and voluntary compatible-kernel migration offer.
  New Zygisk choices and retirement of a currently selected Zygisk are explicitly
  refused until the corresponding executor exists; a warning bit is no grant.
- Cross-boot last-successful hints and complete existing-backend continuity.
- Production preflight schedule/resolution (experimental admission remains off),
  real secondary32 Android launcher, production route/rule/framework scenarios,
  remaining capture/export/agent ownership, notifications and provisioning.

Separate acceptance gates remain actual Android/bionic/provider/SELinux and both
ABI workers, real managers/reboot verification, clean full same-commit bundle,
production signed APK and final startup device regression. None is implied by
host tests, static QEMU units or an ARMv7 compile.
