# Final-byte Core preflight experiment

Date: 4 November 2026. This is an implementation/qualification slice under
[RFC0001](../rfcs/0001-automatic-gki-kmi-selection.md) and
[RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md), not device acceptance.

## Implemented

- Native real-load preparation and experimental preflight share one
  `payloads::frozen::FrozenPayload` owner. It copies a bounded pinned regular file
  without changing its input offset, hashes the actual bytes, and requires
  WRITE/GROW/SHRINK/SEAL memfd seals. Duplicate descriptors and `/proc/.../fd`
  reopening cannot mutate it. No unsealed fallback is allowed.
- Preflight preparation verifies the complete manifest-derived same-LTS candidate
  set, exact copied bytes, both final ELF capability contracts and actual GKI
  loader version records before dispatch. A reduced candidate set, missing
  contract, mutated source or old ELF cannot obtain a syscall by parameter name.
- The isolated Core oracle uses the existing separate journaled preflight budget.
  Running intent sync precedes the syscall; controls are rechecked after sync,
  then after syscall. Boot/ownership/endpoints/kprobes must remain positively
  clear and stable. Missing or changed postconditions retain Unknown, including
  a returned sentinel. Storage uncertainty retains the hold. Duplicate receipts
  are queried and never blindly replayed.
- The actual fixed-parameter syscall entry is compiled only with
  `fixture-oracle`, requires UID0 and the explicit isolated-QEMU boot flag, and
  is excluded from Core/APK production packages. Production selection does not
  dispatch it. There is no new root RPC, force flag or activation following it.
- Typed QEMU `core_preflight` declares both the independent C oracle and Core
  durable-dispatch check. Reports forbid loaded/registered effects and missing
  rows. Input identities retain every staged catalog file. A completed experiment
  is not a compatible-backend verdict; its ledger distinguishes NoSentinel,
  OneSentinel and MultipleSentinels.

## Qualification

- Core/tooling Rust tests: **529 passed, zero failures/skips**. Host all-target
  strict Clippy includes the oracle. Strict production ARM64/ARMv7 Clippy passed.
  Formatting and scaffold/no-su checks passed.
- Tests exercise final-byte retention, write aliases, a synced intent visible
  before the effect, candidate-set tampering, unavailable/changed controls before
  and after sync/syscall, storage crash windows, duplicate retry refusal, global
  loader blocking and ambiguity after two sentinels. Experiments neither consume
  nor grant a real-load budget.
- Reference Image: SHA256
  `c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0`,
  Linux6.1.175 custom QEMU configuration.
- Negative-case session
  `29ebf588e1d1b209145cbb7b791ae2b34d9b0d793b2c70149a4b5acc4bab4525`:
  Core examined the unmodified passive catalog from historical clean full bundle
  `21cd31eedce06cc0e6c48b401da8d425c56c19e5`. Its6.1 payload SHA256
  `15d69c8e5d3108ba98c8c3ecc58c6feb0e0aa3e194e0810f88f7ab191d571cff`
  has a different `module_layout` CRC from this reference Image. Core recorded
  `CandidateRejected(errno=8)` and `NoSentinel`. The independent reference-C
  positive control reached EUCLEAN. Both declared checks completed, zero skips,
  no real load/hooks. This does not reject the payload for other kernels.
- Positive-case session
  `1c6a144c8c2d73e384b7d47246b3f8dc8fee83ca2fd5060795d7a31fcca83ae9`:
  both oracles reached EUCLEAN; Core recorded `Finished(CleanSentinel)` and
  `OneSentinel` from sealed FD bytes SHA256
  `8be313ad60f84a80a27ff9eec5c67d1a503a85cc28a362f9b5df9812c0c36b65`.
  This is the previously qualified **dirty engineering** reference module with
  base `2594bafa93db5d6fdf1941704332d8e04df56619`, explicitly labeled in its isolated
  fixture manifest. It is not restamped as a clean current release. Two declared
  checks completed, zero skips, no real load/hooks.
- The initial same-payload-C run stopped on CRC rejection before Core dispatch.
  Its failed session `554894d980c9ccee7d77383febb5ea67c0c8b62509322d2b250611f0376d2a11`
  is retained. Neither CRCs nor vermagic were edited to pass it.

External evidence is retained under `vpnhide-build-evidence/final-byte-preflight`:
serial/host logs, plans, input identities, exact historical catalogs, the tiny
typed Rust fixture adapter, and source/build qualification. The adapter consumes
the shared artifact types/ELF parser and labels the reference source dirty.
The static oracle/tooling builds retain a clang17 unused `-no-pie` warning; strict
Rust host/Android checks have no relaxed lints.

## Remaining code and actual device gates

Production preparation/dispatch scheduling, reconciliation and typed selection
consumption remain code work. Multiple sentinels must remain ambiguous, and one
sentinel alone proves neither private layouts nor successful activation. Current
production does not enable this experiment. Broader kernel/final-payload/device
qualification, Android SELinux/memfd loading and genuine secondary32 worker
acceptance are unperformed. No phone/provider operation, production signing,
release or new same-commit full bundle is claimed. RPC32/journal15/control2 stay
unchanged; no compatible builtin rebuild is required by these version numbers.
