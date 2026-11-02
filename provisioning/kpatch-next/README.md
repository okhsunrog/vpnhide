# Separate KPatch provisioning work

This component is **not included in the Core ZIP**. No repository was created or
published and no boot partition was read or written on a device.

Pinned upstream bases:

- [KPatch-Next-Module b5612ad](https://github.com/KernelSU-Next/KPatch-Next-Module/tree/b5612adf4d4085a253cb0be30fa1be36e11d09db)
  (GPL-3.0; original notices retained in the patch).
- [KPatch-Next 456744b](https://github.com/KernelSU-Next/KPatch-Next/tree/456744b29efb9989445463ab29e368fa59a103c4)
  (GPL-2.0-or-later; original notices retained in the patch).

The patches apply independently to those exact bases. The engine patch fixes
embedded config extraction: in-memory gzip, CRC/end-marker/size/ambiguity checks,
no predictable `temp.gz`, no partial config stdout on failure, and error propagation
to `dump_ikconfig`. This is required evidence plumbing, not proof that every kernel
with `KALLSYMS_ALL=n` can boot a patched image.

The module patch stages only in `MODPATH`, removes live-module hot deletion,
engine/KPM autoload and failing-KPM deletion, and disables all legacy boot
patch/unpatch/partition-guessing/direct-flash shell entry points. The old WebUI is
not shipped by the offline builder. Installing this tool component does not itself
patch a boot image or activate a VPN Hide backend. Runtime applicability belongs
to the provisioner and Core's authenticated inventory, not `$APATCH` or directory
names. Status uses the exact live Next hello echo.

`build_offline.py` accepts only local inputs and does not access release APIs,
`latest` assets, package registries or executable downloads. Its reviewed module
file allowlist is hash-bound; additional module entry points refuse. The input
manifest has `module_commit`, `engine_commit`, and `tools` records for `kpimg`,
`kptools`, `kpatch`, `magiskboot`. Each record has `bytes`, `sha256`, `source_commit`.
The first three source commits must be the pinned engine commit. All executables
must be ARM64 little-endian ELF; the engine must be the KP2026 payload family.
The caller supplies a trusted build record: a matching JSON hash by itself does
not establish how an arbitrary binary was built. No production tool hashes or
qualification records have been fabricated. Outputs are exclusive and ZIP order,
permissions and timestamps are deterministic.

```sh
python3 provisioning/kpatch-next/build_offline.py \
  --source /path/to/patched-module-checkout \
  --tools /path/to/verified-tool-tuple \
  --manifest /path/to/trusted-build-record.json \
  --output /path/to/new-component.zip
```

The separate Rust package `vpnhide_provisioning` provides actual extracted ARM64
Image inspection and an injected boot-write state machine. Its CLI exposes only:

```sh
cargo run -p vpnhide_provisioning -- inspect-kernel /path/to/extracted/Image
```

Inspection checks a bounded gzip config from those exact bytes, including CRC,
end marker and duplicates; a missing config is Unknown. ARM64 `image_size` is a
memory size and may exceed file length. Linux Kconfig symbol names can contain
lowercase characters. `KALLSYMS=y` must be explicit. `ALL=n` eligibility needs an
engine-hash-bound qualification, not merely source README support. Runtime
Absent/Present/Unknown are distinct; an unknown handshake cannot authorize a patch.

The write model requires full partition backup, exact source/patch hashes,
slot/OTA/snapshot/AVB observations, fresh plan-bound consent, a durable Writing
receipt before dispatch, a final observation after intent fsync, and independent
readback. A changed/reused operation or any post-dispatch uncertainty fences replay.
The model has no concrete device IO adapter; it cannot flash anything by itself.
KernelEvidence and BootPlan cannot be deserialized into forged execution authority.

Executable offline tests apply both patches to pinned checkouts, compile the real
C parser, and exercise gzip corruption/ambiguity/bombs, temp-file symlink immunity,
tool hash/size/allowlist refusals, staging and legacy write refusal. Rust tests
exercise failure/recovery/consent/slot/OTA/backup/readback boundaries. ABI fixtures
used by package tests are never release artifacts.

```sh
VPNHIDE_KPATCH_ENGINE_SRC=/path/to/pinned-engine \
VPNHIDE_KPATCH_MODULE_SRC=/path/to/pinned-module \
python3 -m unittest discover -s provisioning/kpatch-next/tests -v
cargo test -p vpnhide_provisioning
```

Still **unimplemented code**, not device acceptance: a real provisioner runtime
adapter, trusted tool-build/qualification record ingestion, fresh boot extraction
and offline patch/repack executor, pinned partition/update exclusion adapter,
durable backup/receipt implementation, and Android consent/progress integration.
The pure write model is not a substitute for these. Actual engine ALL=n/private
ABI qualification, boot success, real partition writes/readback and manager/ROM
acceptance additionally need controlled device/kernel tests and separate explicit
consent. Production signing needs the maintainer's key.

Publication target is unresolved: which namespace/repository and branch should
hold the corrected KPatch-Next-Module fork, and which target should hold engine
changes? Until that is specified, reviewed patches remain here without creating an
arbitrary external repository.
