# Host tooling ownership and approved migration

The user approved the 13-file migration below after the original read-only
candidate audit, and requested the boundary rules now in `AGENTS.md`. This is an
implementation task. Java signing, unrelated Python generators, release tools,
and the external agent transport remain adapters in their existing languages.

| Approved scope | Current owner and status |
| --- | --- |
| `scripts/kmod_elf.py` + `test_kmod_elf.py` | Replaced by `artifact-model::elf` and `vpnhide-tooling elf`; Core uses the same bounded ELF verifier. Independent raw ELF and saved golden fixtures remain. |
| `scripts/payload_bundle.py` + `test_payload_bundle.py` | Replaced by `artifact-model::{manifest,archive}` and `vpnhide-tooling bundle`. Build scripts only invoke the typed stamp operation. |
| `scripts/core_descriptor.py` + `test_core_descriptor.py` | Replaced by shared package formats, typed build compatibility and Rust descriptor/signing orchestration. The Java keystore provider is unchanged. No regex reads of source constants. |
| `scripts/public_distribution.py` + `test_public_distribution.py` | Replaced by `artifact-model::distribution` and `vpnhide-tooling distribution select`; only the paired APK, full Core ZIP and descriptor are public assets. |
| `scripts/update-json.sh` generation | Thin bootstrap retained; Rust owns validation and the Core-only update feed. Legacy backend feeds remain frozen compatibility data. |
| `kmod/test/{init,run,run-kpm}.sh` | Replaced by typed Rust host/guest plans, bounded direct execution, complete reports and RAW/preservation classifiers. Minimal PID1/host shell bootstrap and real bionic C probes remain. KPM uses two independent boot reports. Linux/QEMU results cannot replace Android acceptance. |
| `scripts/clean-device.sh` | Replaced by a thin entrypoint to the typed resumable Core reset client. Offline plan by default; explicit serial/boot, native peer-UID check, durable intent/receipt and optional post-confirmation APK removal. No journal deletion or offline writer; destructive cold rescue remains pending. No device execution performed. |

The static host CLI is an internal build artifact consumed by minimal DDK jobs.
It links portable formats/contracts, not the root daemon or device runtime. CI
builds it from the same checkout before producer jobs and keeps it outside the
public asset allowlist. Local container builds mount the corresponding portable
executor. Runtime still checks pinned real files and publisher authentication;
a host inspection report does not establish runtime eligibility.

Production Android entrypoints remain compiled Core: `service.sh` only execs
Core, `customize.sh` installs the ABI binary, and manager recovery installers are
compatibility shims. This migration does not relax production scaffold/no-su
checks or turn the external HTTP/ADB bridge into a root endpoint.

The separate 32-bit Zygote loader bootstrap gap remains runtime implementation
work. Same-package instrumentation stops the main package; `--no-restart` cannot
fork a new ABI process. It needs a genuine Android process and explicit install
handling, not root-setuid evidence or an ABI guess.
