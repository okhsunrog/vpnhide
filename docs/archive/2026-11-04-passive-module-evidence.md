# Passive module evidence: local qualification

This slice belongs to the unpublished local backend-policy branch under
[RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md). It is not Android/kernel
acceptance, a published package, or permission to push the new policy to main.

## Implemented call path

`Prepared::from_bundle` → `payloads::device::observe` →
`standard_modules::observe` → `selection::assess_projected` →
`kmi::resolve_with_modules` consumes one common artifact-model ELF metadata owner.
The existing capability verifier uses the same bounded `.modinfo` grammar.

The read-only observer scans fixed vendor/system/odm module locations, at most one
nested directory level,512 entries,16 MiB/file,64 MiB total and32 retained records.
Its two-second deadline is cooperative between entries, not a hard IO timeout.
It pins/type-checks files before opening, refuses links and writable/foreign-owned
directories/files, and hashes actual ELF bytes. It requires a matching live sysfs
GNU build ID, matching source version when supplied, no forced-load taint and
stable file metadata plus reobserved live identity. Missing/incompatible metadata,
compressed files and unavailable runtime notes cannot supply corroboration.

At least two distinct normalized module names and file hashes with one consistent
Android/LTS hint can narrow the verified manifest candidate set. A repeated copy,
one KernelSU Next module, a filename, uname hint alone or an incomplete scan cannot
settle a tie. Observed conflicts survive incomplete inventory and contradicting
uname evidence. The final artifact hash and import/version CRC checks remain
unchanged; no bytes/vermagic are rewritten and no loader trial is performed.

Unpublished architecture input now records observed uname machine separately
from Core ABI. Unknown/non-AArch64 machine refuses new ARM64 ko/KPM admission.
The existing conservative refusal by a32-bit Core remains; this does not add an
ARM32 kernel port or prove a32-bit Android worker exists.

## Cloud checks

-404 Core library tests, no failures/skips.
-34 artifact-model library tests with archives/fixtures, no failures/skips.
- Strict host all-target Clippy including fixture oracle, strict ARM64/ARMv7 Core
  library Clippy, formatting and whitespace checks.
- Retained logs: `/workspace/vpnhide-build-evidence/backend-policy-passive-modules/`.

Tests include literal independent ELF notes, duplicate/absent/control-character
metadata, every truncated ELF/note prefix, build-ID mismatch, source mismatch,
forced/unloaded state, symlink files, writable directories, inventory limits,
duplicate copies/names/hashes, conflicting and partial inventories, manifest order
reversal, selector integration and unknown/non-AArch64 machine.

## Remaining integration

Hard child-process observation deadline and durable/public selection evidence
remain code work. Final-file preflight dispatcher/recovery remains separate code
work; no production experimental gate was enabled. Provider lifecycle, old-Zygisk
continuity/offer and explicit backend settings are not completed by this observer.
Real vendor module inventory, kernel-private ABI, Android SELinux and device
behavior require later device acceptance. See [ROADMAP](../../ROADMAP.md).
