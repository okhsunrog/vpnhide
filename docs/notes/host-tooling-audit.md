# Host tooling ownership audit

Read-only audit requested after the unified-distribution cleanup. This is a
candidate list, not authorization or implementation of a broad Rust rewrite.

Production Android entrypoints already delegate runtime behavior to compiled
Core: `core/module/service.sh` only execs Core, `customize.sh` places its ABI binary,
and manager `META-INF/.../update-binary` files are installer compatibility shims.
QEMU guest shell scripts and host ADB tools are not Android production daemons.

| Candidate | Domain ownership and proposed boundary |
| --- | --- |
| `scripts/kmod_elf.py` and `crates/core-daemon/src/payloads/elf.rs` | Best shared-library candidate: security-sensitive ELF capability/parameter verification has two implementations. A portable Rust library plus host CLI could own it, preserving executable shared fixtures and independent imports/CRC reports. No port done by this audit. |
| `scripts/clean-device.sh` | It duplicates reset paths and can delete the private journal. Ordinary reset belongs to typed Core recovery. A separately labelled cold-lab wipe may remain a host adapter; never use journal deletion to run an older Core. Script was not executed. |
| `scripts/network-view-probe.py` | Root app_process followed by setuid is manual forensics, not a genuine Zygote app-worker acceptance result. Production orchestration/verdict belongs to the already approved autonomous Core worker; retain this tool only with its narrower evidence scope. No new production path was added. |
| KPatch boot extraction/patch/write domain | Belongs in the already approved separate Rust provisioner. Keep dangerous old shell-write entries disabled. This does not require porting the Python offline tool packager. |

Keep host orchestration in Python/shell unless a concrete domain duplication
justifies a later change: component build scripts, `build_lib.py`, payload assembly,
keystore/provider integration, public asset selection, code generation, help
indexing, changelog/release/version tools, update-feed generation, stats and smoke
tools, QEMU harnesses, kernel-tree integration and `tools/agent-mcp/server.py`.
The last is an ADB/MCP/HTTP transport adapter; moving it wholesale into a root
endpoint would change the trust boundary rather than remove duplication.

The separate 32-bit Zygote loader bootstrap gap is runtime implementation work,
not a reason to rewrite these host tools. Same-package instrumentation stops the
main package, while --no-restart cannot fork a new ABI process. Its solution needs
a bounded real Android process path and explicit installation handling; an ABI
guess or root-setuid worker cannot supply that evidence. See the reviewed
[AOSP instrumentation lifecycle](https://github.com/aosp-mirror/platform_frameworks_base/blob/1cdfff555f4a21f71ccc978290e2e212e2f8b168/services/core/java/com/android/server/am/ActivityManagerService.java).
