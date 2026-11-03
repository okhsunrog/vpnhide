# GKI modpost qualification experiment — 3 November 2026

Dated build evidence, not a release or Android acceptance. The source baseline
was `ad0b70723d906cbe1bd6dfe17028ae2a343f7f70` plus the uncommitted qualification
fix. These engineering producer ZIPs explicitly record dirty provenance; they
must not be described as clean same-commit release artifacts. The clean rebuild
is a separate gate in [ROADMAP](../../ROADMAP.md).

## Failure and repair

Fresh builds of all seven pinned DDK20260313 payloads initially had empty
`__versions`, despite MODVERSIONS and populated input `Module.symvers`. The DDK
`modpost.c` comments out both export checking and `s->module = exp->module`.
Rebuilding its prebuilt tool without restoring those checks would preserve the
bug. The Rust host adapter restores exactly the two recognized statements in a
private copy and recompiles the checker from its host C sources/headers. The
installed DDK source, ABI headers, configuration and symvers are not modified.

The Kbuild integration also matters: old kernels store checker and flags in
`MODPOST`; newer ones use separate `modpost-args`, with executable assignment
before or after the external Makefile include. A command-line `MODPOST` override
lost old-kernel flags. The adapter now replaces only the executable, preserving
kernel-owned recursive configuration, target and symbol inputs. Executable tests
cover those layouts and reject unknown command forms.

The final ELF inspector independently requires the init/preflight markers and
normal version records. Assembly, descriptor creation and Core new-activation/
replacement admission reject incomplete GKI bytes even when hashes/signatures
are valid. Old-source catalogs remain readable/fenced for migration. Generic
non-MODVERSIONS QEMU payloads retain their separate research scope.

## Actual engineering outputs

Each final object passed `elf require-gki-contract`; every import has a CRC and
`module_layout` is present. Zero is a valid CRC value; absence is not.

| KMI | ELF bytes | Imports | CRC records | SHA256 |
| --- | ---: | ---: | ---: | --- |
| android12-5.10 | 101112 | 46 | 47 | `686e986b1a6fc5d73cb02fe8e2bc7a3c287d62b93eb55024fe9e261795e3aa31` |
| android13-5.10 | 102960 | 47 | 48 | `b0efb4bb8ee1d3834a6dd6d2c9dadf323ca59dfb7abd65a672465b049280978e` |
| android13-5.15 | 108464 | 48 | 49 | `78a105e1f8c0894f86025da35a04dff8b0338796c44255b651d8fa7d20426c09` |
| android14-5.15 | 130032 | 48 | 49 | `201f78e74c2ff9475e22def1e4945b1913fba333df94c7f533280650bd2acb89` |
| android14-6.1 | 103088 | 46 | 47 | `858d3a6800bb54c4164be4ffbabffe12ea9ab59103b45e1d03dc96af96e25cb7` |
| android15-6.6 | 88400 | 45 | 46 | `7700a88f2fa63926e53236b9a8d349fc45883d62005215753b31c0c05a6bdb9b` |
| android16-6.12 | 108760 | 45 | 46 | `1a57d2c4462ddaea6d2496c264eac7a816065cb459cc457c32f918f14207af51` |

The pinned image digests, original/qualified modpost source hashes, compiler
logs, actual ELF inspection reports, producer ZIPs and same-LTS comparison JSON
were retained under the build evidence directory
`/workspace/vpnhide-build-evidence/gki-qualified-engineering` in the execution
workspace. For Android16/6.12 the pinned image is
`sha256:4dd088dc94b391c926cf09215e0a5958f3dfe7d8eeed3616f9041224f85b1d1a`;
original modpost source SHA256 is
`39288549637466430f94c3f7b48e0372d7cb0f23911bbe413dc3f3e05be7e31e`,
qualified source SHA256 is
`5abfa38181840d8a32cd792df0aab1c8df2bbb8b2a0038bf9c38aea1af1c6477`.

## Same-LTS evidence and limits

Android12/5.10 versus Android13/5.10: 46 common version records, 18 changed CRCs;
imports also differ (`__stack_chk_guard` versus `bcmp`/`strlen`).
Android13/5.15 versus Android14/5.15: identical import sets, 49 common version
records, 18 changed CRCs. Neither side has missing import CRCs. Thus loader
version checks can distinguish these payload generations even for identical
imports; choosing the first filename is unjustified.

Complete records do not prove their values match a device kernel, private
structure ABI, early-init safety on all kernels, or effective protection.
Negative errno alone is not that proof. Production experimental KMI admission
stays off; no real device/provider/load action occurred in this experiment.
The earlier structurally assembled bundle containing unversioned payloads remains
quarantined and is not a production-loadability result.

## Verification of the code slice

Core's 364 unit tests and its process/integration suites, 29 portable artifact
tests, host CLI/signing tests, and the Kbuild executable regressions passed.
The new signed-target regression rejects hash-valid CRC-free replacement while
still fencing the old source. Strict host all-target/all-feature Clippy and
ARM64/ARMv7 Core all-target Clippy passed, as did Rust formatting, the pinned
Python lint/format checks and producer adapter regression. These are scoped
cloud results, not the final startup regression or a production-signed APK.

## Clean post-commit rebuild

The separate clean checkout of `12c78631aad9205458609a4885dcebbb143082e6`
subsequently rebuilt both Core and Zygisk ABIs, KPM and all seven qualified KMI
payloads. Every producer reports that exact commit and `dirty=false`. Assembly
passed with 20 entries, 10 payloads, exactly one passive Zygisk pair and no main
Core discovery directory. Full ZIP: 4,306,807 bytes, SHA256
`39f93785b531085053290db5f3769b000df2c67c61ae9d58559494d43286e04e`.
Actual native raw/compressed entry inventory and retained logs are in
`/workspace/vpnhide-build-evidence/arm-release-12c78631`.
This supersedes the earlier build/provenance gate for that source checkpoint,
not production signing, device acceptance or a final rebuild after subsequent
implementation work. No production key or device was used.
