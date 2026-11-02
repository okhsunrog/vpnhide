# RFC 0003: conditional Zygisk discovery owned by Core

Status: **Reopened for lifecycle review** (2026-11-02). The maintainer first
approved implementation, then explicitly paused new conditional publication and
boot-bound activation policy pending a full lifecycle matrix. ZIP deduplication
remains required; final placement and activation policy are under review. The
package-only implementation already committed is preserved, not declared a
completed Zygisk runtime. See [the current roadmap](../../ROADMAP.md).

The sections below preserve the previously approved proposal and its constraints;
they are not authority to continue the paused activation redesign. Review covers
fresh install,1.x migration,APK-only and Core patch/minor updates,kernel changes,
builtin appearance,fallback and boot failures, including multi-week uptime and
avoiding unnecessary repeated reboots. A remembered successful backend is a
proposal, not authorization or proof of current applicability. Mapping and
installed hooks remain separate; safe coexistence cannot be assumed.

## Reopened review notes

The following are proposals, not approval to implement new transition policy:
stick with a healthy backend during uptime; validate a previous successful backend
as a next-boot hint; combine migration/update preparation with an already needed
boot; offer Auto and an exact Manual backend without hidden fallback. Partial
useful coverage and confirmed total failure need different product decisions.
Kernel-to-kernel transitions may require a reboot; that does not authorize an
automatic reboot or a chain of automatic attempts.

The default UX must minimize cognitive load: routine partial coverage must not
produce a backend-choice popup. Advanced Manual control is under discussion;
automatic recovery still needs a complete safe decision table. Evaluate missing
hooks against desired features and demonstrated compatible benefit, not merely a
nonzero status. No blind trials or unknown hook overlap follow from this principle.

**Confirmed invariant: Core never reboots the phone automatically.** Preparation
and the user's decision to reboot are separate. No reboot timer is allowed.

Current selection uses fixed-priority eligible candidates and frozen plans.
Automatic progression requires durable no-effect evidence and clear ownership;
unknown load effects stop it. Diagnostic failure does not initiate fallback.
No Auto/Manual setting, previous-backend hint or projection executor exists.

The health review must separate load, config delivery, required hook coverage and
behavioral proof. Current resident admission in
[`bundled.rs`](../../crates/core-daemon/src/native_boot/prepared/bundled.rs)
collapses a nonzero status error into an empty hook mask, while KPM configuration
can accept PartialHooks and the read model presents a warning. This inconsistency
is unresolved; neither blanket nonzero-error fallback nor silently accepting any
partial mask is an approved fix. Missing process-coverage proof is separate again.

The earlier proposal below is preserved for review, including constraints now
being reconsidered. Its boot-bound descriptions are not new implementation
permission.

## Decision and scope

Users install one universal Core ZIP. It contains exactly one passive pair:
`payloads/zygisk/arm64-v8a.so` and `payloads/zygisk/armeabi-v7a.so`. The main
`vpnhide_core` module has no top-level `zygisk/` directory. When needed, Core may
make a verified ordinary copy into its separately owned discovery projection,
provisionally `vpnhide_zygisk_runtime/zygisk/`. Deduplication concerns the installer
archive, not installed disk space. Do not add hardlinks, symlinks or moves to
avoid the legitimate runtime copy.

The projection is a representation of Core state: no daemon, service script,
post-fs-data activator, selector, independent policy, download, version channel or
separate user ZIP. Only Core decides, journals and publishes it. Root managers
may show this internal module entry; it must respect owner disable/remove and
must not interfere with LSPosed or any other module/provider setting.

The dependable baseline is a boot-bound transition with a typed `PendingRestart`
outcome. Hot refresh is a later provider/version/ABI adapter, not an optimistic
CLI call. Core does not restart zygote/provider or reboot the device automatically.
A future certified adapter needs exact catalog/image/ABI generation proof and
safety for other modules' outstanding handles. `rearm` exit0/AlreadyHooked is not
that proof; `ignore-disable` is prohibited.

## Why publication is independent of authorization

Authorization inside a library prevents hooks after `dlopen`; it cannot prevent
that first mapping. Core's current selection waits for PackageManager, which can
be later than provider catalog creation. Missing/invalid current-boot authority
must continue to deny hooks, and non-target/server callbacks request dlclose,
but neither is a pre-discovery gate.

Magisk33bfff7ce750c27f157ddb4580e36bf842d6d95b additionally omits an entire module
with `zygisk/` when global Zygisk is off. A permanent directory under Core can
therefore omit Core's service too. Keeping the passive pair outside discovery
removes this coupling. NeoZygisk's snapshot refresh has outstanding-index hazards;
ZygiskNext documents preload behavior and has no certified adapter here.

## State and durable boundaries

Model desired backend, boot plan, projection per ABI, observed provider catalog,
hook authorization and consumed process generations separately. Files prepared,
manager staging accepted, new boot observed, provider discovered and hooks applied
are distinct outcomes. Never set `active` merely because a file was copied.

Preparation pins the active Core catalog and exact payloads. Journal publication
intent before effects, build both required ABIs outside scan-visible paths,
verify bytes/ELF/root ownership, fsync, then use a supported manager staging path.
No ad hoc mkdir into the active module tree. A missing manager contract becomes
an explicit unavailable/action-required result. Unknown effects hold the same
operation; do not blindly install again or allocate another load attempt.

Loader verification must split the authoritative Core catalog directory from the
recipient projection directory. Keep both device/inode identities and exact
manifest/payload hashes. Do not replace the current identity checks with a path
prefix or arbitrary `vpnhide_*` name. Legacy standalone recipients remain only
in explicit migration/revocation adapters. Authorization binds boot, generation,
policy and recipient; observed FD/memfd bytes and both required ABIs remain
mandatory. A 64-bit diagnostic Service cannot supply 32-bit proof.

The discovery-only bootstrap stage has no hook authority. If provider acceptance
is unavailable in the current boot, retain the operation as PendingRestart. After
a verified next-boot boundary, prove actual discovery before authorizing hooks.
Unknown real kernel load prevents parallel Zygisk activation. Possible Zygisk
hook exposure still forbids a same-boot transition to a kernel backend.

## Kernel mode and boot ordering

Kernel mode must exclude our discovery payload **before** provider scan. Late
unlink does not remove retained FD/memfd, and a service script scheduling race is
not a pre-scan contract. A boot plan that may have exposed the projection cannot
silently become a kernel boot; withdraw authority/discovery and use the next
verified boot. After sudden power loss, incomplete publication/removal, Core
update, owner disable/remove or a missed early deadline, reconcile exact owned
state and preserve Unknown/PendingRestart instead of claiming zero-load.

A supported manager integration must establish its installation/staging and early
ordering contract, including Core disabled or removed. If that cannot be proved,
strict exclusion is unavailable for that combination. Do not infer it from an
absence in a late maps snapshot. An early entry, if required, executes the same
bounded Core reducer/journal/owner lock; it must not wait for PackageManager,
network, UI or workers. No second shell selector or independent activator.

## Update, withdrawal and recovery

First installation has no prior provider image; migration from the old Core layout
may have one. Stage a verified replacement before retiring old paths. Keep journal
budgets/holds across schema migration, and verify actual active/running generation
after reboot before native boot admission. Manager staging is not active runtime.

Withdraw hook authorization before discovery removal. Remove only a projection
whose ownership, generation and directory identity are established by Core's
receipt. Unknown or manually disabled entries are not adopted or reenabled.
Already hooked apps/companions may remain resident until their own lifecycle
boundary; withdrawal cannot undo old hooks or an earlier mapping. Never kill a
shared companion or change a foreign module for cleanup.

No real provider settings, device installs or disruptive restarts are authorized
by this RFC implementation approval. Device acceptance requires a separate step.

## Verification

Cloud executable checks cover archive dedup, main Core service enumeration with
Magisk Zygisk off, both ABI copies as one generation, missing/forged files,
symlinks/inode changes, stale catalog/authorization/ACK, owner disable/remove,
crash at each durable boundary, boot ordering, first install and old-layout
upgrade/downgrade. RPC/FFI/Kotlin and export states change together where needed.

Device gates cover the installed manager/provider versions, pre-scan exclusion,
actual catalogs/mappings/companions, both Zygotes, old game processes, reboot and
interactions with LSPosed/other modules. A late absence observation is weaker than
proof that the library was never mapped. Unknown is not PASS.

## Research inputs

The maintainer reviewed `zygisk-conditional-loading-design.md` (2026-11-02). The
accepted decisions above supersede that report's proposal-only status and its
suggestion to retain permanent baseline publication. Source anchors:

- [Magisk module collection](https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/native/src/core/module.rs#L701)
- [Magisk service list](https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/native/src/core/bootstages.rs#L167)
- [NeoZygisk snapshots and handles](https://github.com/JingMatrix/NeoZygisk/blob/edfa820a79d8f0ab39a29c5c2be0f03a6fc58851/zygiskd/src/zygiskd.rs)
- [NeoZygisk rearm](https://github.com/JingMatrix/NeoZygisk/blob/edfa820a79d8f0ab39a29c5c2be0f03a6fc58851/loader/src/ptracer/zygote_abi.cpp#L237)
