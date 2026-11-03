# Authenticated Core replacement

Core now admits explicit replacement commands on its durable command writer.
This covers updating an existing complete Core installation; missing-Core
bootstrap and manager/SAF handoff for incompatible RPC versions remain separate
work.
Compatibility versions are owned by [build-contract](../crates/build-contract/src/lib.rs)
and the shared protocol crate. APK/Core IPC and journal compatibility do not
require rebuilding an otherwise compatible user builtin kernel. The executor can stage an update through a qualified manager; it never reboots
or writes a boot image.

The existing app socket authenticates its peer UID and directory ownership. It
does not verify APK signing certificates. An app-supplied ZIP hash or an unsigned
payload catalog therefore cannot authorize new root executable bytes.

Trusted CI exports only the public identity of the existing Android keystore
before building Core. `VPNHIDE_CORE_TRUST_FILE` pins that identity in the Rust
package-trust library; it is a build input, with no runtime environment override.
`VPNHIDE_BUILD_SEQUENCE` is the same workflow run number multiplied by1000 plus
its bounded attempt number. Installation authority requires both a nonempty key
pin and a positive compiled sequence. PRs receive neither installation authority
nor signing secrets. An unconfigured developer build remains usable but cannot
prepare privileged package replacement.

After the complete seven-KMI/KPM/two-ABI Zygisk bundle is assembled and checked,
the same existing private key signs an exact bounded descriptor prefixed by
`VPNHIDE-CORE-REPLACEMENT-v1\n`. Java KeyStore/Signature never exports the private
key and never generates a fallback identity. RSA2048–8192/SHA256 PKCS1 and exact
P256/SHA256 or P384/SHA384 ASN.1 signatures are supported. Rust verifies against
only its compiled public identities using ring; envelope-supplied keys and
algorithms are not accepted. See [Java Signature](https://docs.oracle.com/javase/8/docs/api/java/security/Signature.html)
and [ring signature verification](https://docs.rs/ring/0.17.14/ring/signature/index.html).

The descriptor binds module ID, exact ZIP hash/size, source commit, signed build
sequence, RPC and journal compatibility, frozen kernel control version and
post-customize daemon hashes for both ABIs plus the exact payload-manifest hash.
The APK embeds that signed target after Core assembly and before APK signing;
the descriptor also accompanies the Core Actions artifact and future release
asset set. No release is created by this change. The nonexistent Core updateJson
URL was removed from CI; a mutable latest URL is not an authenticated target.

Preparation requires the expected target hash/commit and retained accepted floor.
An older signed sequence is refused. Equal sequence is idempotent only for the
same previously accepted package. The compiled current sequence supplies a lower
bound even before the first maintenance record exists. Compatible user builtin
kernels do not need rebuilding because APK/Core or journal versions changed.

Core accepts source bytes only from the fixed authenticated app directory under
`files/core-maintenance/<authenticated-hash>.zip`. Descriptor-relative nonblocking,
no-follow opens reject symlinks, FIFOs, other nonregular files and hardlinks.
It copies into the journal parent's private `maintenance-packages` directory,
checks source identity/change metadata, syncs and verifies the private copy, then
publishes with rename-NOREPLACE and verifies again. A retained read-only CLOEXEC
descriptor binds the prepared result to those bytes. The active module directory
is never a staging destination. Existing valid private bytes can be reused even
if the app source has subsequently changed or disappeared.

An exclusive file lease serializes preparation across independent instances.
Only private inert `copy-<operation>` leftovers are recovered under that lease;
final packages are retained for the durable executor. At most two bounded final
packages are admitted, each at most256MiB. With no retained replacement and an
exclusive preparation permit, authenticated preparation releases only known inert
prior packages, retaining the accepted floor package and incoming target. An
active operation's package cannot be pruned. A full store otherwise refuses. Unknown files,
exposed directories, damaged stored packages and uncertain storage stay errors.

`package::FloorStore` now retains accepted sequence/hash state in the private
journal parent, outside both replaceable code and pruned receipts. Preparation
requires its exclusive lease and reads the real floor rather than accepting a
caller-supplied zero. Copying a package remains inert: it does not advance the
floor. A separate acceptance API takes the private-copy token for the same
persistent directory and rechecks its ownership, identity, length and actual hash
without changing the retained reader's offset. A changed private copy cannot
advance the floor. The shared trust policy refuses lower sequences and a
different package at an equal sequence. Malformed state, exposed ownership/modes,
symlinks, special files and hardlinks fail closed; absence alone is initial zero.
Atomic acceptance syncs the temporary file, renames and syncs the parent.
Post-rename sync failure is explicitly uncertain and can reconfirm the same
package without resetting the visible floor. Only bounded root-owned inert
temporary files are recovered under the lease.

## Durable command and manager dispatch

`StageCoreReplacement` carries a bounded signed envelope and expected target,
not a client-selected path, URL, executable or manager. Preparation runs outside
the writer: publisher authentication, anti-rollback, private-copy/full-ZIP
validation, the actual live/running image fence, manager contract and positively
identified legacy inventory. The prepared token retains those exact inputs.

The writer syncs accepted work before acknowledging the operation, accepts the
private package floor, then syncs dispatch intent before spawning the retained
manager ELF. A disconnected caller does not cancel accepted work. The same
operation ID deduplicates; no error/timeout/unknown dispatch is automatically
replayed. `replacement` is retained independently of receipt pruning, while the
last replacement receipt is pinned. FullReset/Cleanup refuse retained replacement
work. Unverified generations hold ordinary config/runtime Repair writes as well;
status reads remain available, so Repair cannot bypass the generation gate.

Manager discovery uses fixed root-owned paths, ELF identity, bounded version/help
queries and, for KernelSU/APatch, actual module-list evidence. Multiple distinct
managers or effective custom metamodule install/uninstall scripts refuse
preparation. The platform `/data` ancestor has Android's system ownership and
mode ([AOSP init](https://github.com/aosp-mirror/platform_system_core/blob/android-16.0.0_r1/rootdir/init.rc#L594)); the privileged `/data/adb` boundary and executable
ancestors remain root-owned and non-writable. Both the original discovery alias
and retained resolved executable are rechecked before effects. Commands use a cleared environment, explicit `BOOTMODE=true`, a
retained executable and the private package FD. There is no shell or su executor
in the APK/Core adapter. Manager install execution is bounded to120seconds with
64KiB output; read-only queries use3seconds. Neither exit0 nor installer output
establishes a staged image.

After dispatch, Core verifies the full staged image and signed daemon identity
for the actual installed ARM ABI, rechecks the live/executing image and prepared
manager/legacy sources, journals retirement intent, then creates only the fixed
legacy modules' disable/remove markers in both live and staged locations. The
legacy module ID and author plus a complete retained file/tree fence are required.
Unknown or changed modules refuse. Old code is not deleted before replacement
readiness; markers do not unload existing hooks. PendingRestart confirms staging,
not running protection, and never triggers a reboot.

Before native startup, read-only reconciliation verifies boot identity and the
actual `/proc/self/exe` inode. In the same boot, PendingRestart can retain the
proven old runtime. On a later boot, the compiled trust pin authenticates the
retained target again, the accepted floor must match, and the active signed image
must match its staged content and executing inode. Pre-boot legacy retirement
must have been recorded (or the prepared inventory must have been empty), with
positive current absence/retirement readback. Missing/wrong proof blocks native
startup rather than selecting another backend. A pre-dispatch crash can release
its hold only after unchanged live/running/legacy and absent-staging proof.
Later proof releases held resources without rewriting a historical Unknown
receipt. No manager retry or reboot occurs during reconciliation.

The APK exposes a document chooser only when it has an authenticated embedded
Core target and a current compatible session. Its bounded SAF adapter copies
exactly the target size; Core repeats authentication and checks the real bytes.
Typed progress distinguishes preparing, PendingRestart, Verified, known refusal
and Unknown; loss of current state withdraws running-generation confirmation.
Save/Cancel and config drafts are unchanged. Unsigned/unconfigured developer
builds remain usable but do not expose privileged replacement authority.

Remaining code: missing-Core rootless bootstrap/manager handoff, automated
artifact retrieval/update scheduling, and richer actionable manager refusals.
Actual manager/fork/SELinux/reboot acceptance, production CI signing and the
final same-commit full bundle remain separate gates. Host tests use independent
synthetic payloads and test-only publisher identities; they do not install an
Android manager or establish phone acceptance.

Manager readback must not use active module.prop as installed generation proof.
Upstream installers update that file while staging new code. Pin the original
live executable/catalog, explicitly request BOOTMODE staging, reject hot-install
and effective third-party metamodule installers, and independently verify the
staged daemon/catalog before legacy removals. Magisk's global remove-modules CLI
must never be used for per-module cleanup. These adapter contracts do not certify arbitrary forks.

Reference sources checked for that staging boundary are
[KernelSU module dispatch](https://github.com/tiann/KernelSU/blob/28db74f6f4692e97006ce9ebd8b19b9cc614b5b7/userspace/ksud/src/module.rs),
its [installer](https://github.com/tiann/KernelSU/blob/28db74f6f4692e97006ce9ebd8b19b9cc614b5b7/userspace/ksud/src/installer.sh),
[APatch's installer](https://github.com/bmax121/APatch/blob/e8e4651c16fb5075882a25813c8732285fdb3505/apd/assets/installer.sh),
and [Magisk's installer functions](https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/scripts/util_functions.sh).
All choose `modules_update` through `BOOTMODE` and update live module metadata
before the code swap. KernelSU also has an explicit hot-install dispatch and
effective metamodule installer selection. These source observations are inputs
to the bounded adapter implementation, not device/fork qualification or proof
that an exit code confirms a staged/running replacement.


The shared Rust host tooling now forms descriptor schema2 from the exact final
package bytes and typed compatibility constants. It includes service, sepolicy
and module.prop identities. Core's post-customize image fence checks the selected
ABI daemon, manifest, complete passive payloads and execution identity;
manager staging markers cannot be accepted as a running replacement. Schema1
install descriptors are rejected without changing the public trust-store format.
The portable `artifact-model::core_layout` owns the permitted Core producer,
final ZIP and installed-file namespaces. Assembly, descriptor creation and staged
image verification refuse extra boot scripts, mount trees and payload members.
Device inspection walks retained directory descriptors, bounds entries/deadline,
and refuses symlinks, special files and writable/unowned members. Known installer
metadata may remain; an empty regular `update` marker is inert staging metadata.
A verified image retains this layout requirement on subsequent unchanged-image
checks, while historical source inspection remains available for migration.
New signed replacements must have no main-Core autoload directory; old-source
mirror files remain fenced for migration. Final Zygisk placement/activation is
under renewed review in RFC0003. The replacement executor uses these checks; missing-Core bootstrap remains
separate. No backend publication or automatic fallback policy is introduced.

New GKI payload admission also checks the exact ELF's normal module-version
records: `module_layout` and every imported symbol must have a CRC. Trusted
signatures/hashes and declared init capabilities cannot substitute for those
records. The kernel loader still validates their values; this is not private ABI
proof. Historical source fencing deliberately accepts older catalogs so an
unqualified old generation can be retired safely rather than blocking recovery.
The [build qualification](../kmod/BUILDING.md#export-and-loader-version-qualification)
uses the same portable Rust inspector; generic non-MODVERSIONS test kernels have
a separate scope and cannot qualify a new GKI replacement.
