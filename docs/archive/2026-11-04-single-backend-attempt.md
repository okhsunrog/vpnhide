# Single backend attempt: local policy implementation slice

This is engineering evidence for the local `review/backend-policy-2026-11-03`
branch, not a main publication or device acceptance. Main publication requires
separate maintainer confirmation under [RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md).

## Implemented scope

- Fresh catalog Auto chooses an eligible Builtin, then KMOD. KPM/Zygisk
  eligibility remains inspectable but does not make them automatic choices.
- Prepared freezes one candidate. The successor queue, successor preparation,
  executor loop and restart logic that skipped a rejected family are removed.
- Known no-effect rejection returns the selected failure. Its private proof and
  attempt budget remain durable; a later admission cannot replay that rejected
  family or continue the old automatic chain.
- Intent/completion writes, pinned catalog validation, pre-dispatch ownership
  checks and all existing journal uncertainty/exposure guards remain.

Regression tests exercise known rejection with successful/failed journal writes,
unknown rejection, replacement before dispatch, ownership appearing during the
intent write, and restart after durable rejection. Policy projection still carries
the same complete multi-profile UID set. Independent candidate tests preserve
needed-ABI/default-mask restrictions and check that otherwise eligible Zygisk is
not automatically selected.

## Verification

The final source passed 398 Core library tests with no failures or skips and strict
host all-target Clippy including the fixture oracle, and strict ARM64/ARMv7 Core
library Clippy. Rust formatting passed. Logs are retained under
`/workspace/vpnhide-build-evidence/backend-policy-single-attempt/` in the cloud
workspace. Compilation does not prove Android runtime coverage.

No RPC, journal, diagnostic-contract or backend-wire version changed. No phone,
provider setting, release, production signing key or main push was used.

## Remaining code

This slice does not complete actually-working backend continuity across upgrade,
explicit backend intent/settings, informed new Zygisk selection, post-update
migration offers, passive standard-module inventory, or final-file preflight
dispatch. Existing legacy adoption remains an isolated compatibility path. These
are implementation tasks, not merely device acceptance. See [ROADMAP](../../ROADMAP.md).
