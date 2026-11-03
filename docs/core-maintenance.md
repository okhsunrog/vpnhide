# Authenticated Core replacement preparation

This describes implemented preparation, not a completed install/update flow.
Compatibility versions are owned by [build-contract](../crates/build-contract/src/lib.rs)
and the shared protocol crate. APK/Core IPC and journal compatibility do not
require rebuilding an otherwise compatible user builtin kernel. No device manager operation or boot write is performed by these APIs.

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
packages are admitted, each at most256MiB. A full store returns a distinct refusal
and does not silently garbage-collect another operation's package. Unknown files,
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

This component has no service/command admission caller yet. The common executor
must invoke acceptance after durable user-command admission and before manager
dispatch; it must separately retain active work and verify the complete staged
replacement. A prepared token or retained floor alone never authorizes executing
an installer or declaring the new image active.

Still to implement: expose this preparation through the common durable
maintenance plan, wire floor acceptance and retain active work outside receipt
pruning, manager capability/dispatch/readback, positively identified obsolete
companion retirement, reboot verification before native startup, app progress and
rootless bootstrap/SAF/manager handoff. These are cloud implementation tasks.
Actual manager/SELinux/reboot acceptance, production CI signing and a distributable
same-commit final bundle are distinct external or build gates; passing these
host preparation tests does not close them.

Manager readback must not use active module.prop as installed generation proof.
Upstream installers update that file while staging new code. Pin the original
live executable/catalog, explicitly request BOOTMODE staging, reject hot-install
and effective third-party metamodule installers, and independently verify the
staged daemon/catalog before legacy removals. Magisk's global remove-modules CLI
must never be used for per-module cleanup. Source pins and adapter details will
be recorded with the implementation, rather than assuming all forks match.

Reference sources checked for that staging boundary are
[KernelSU module dispatch](https://github.com/tiann/KernelSU/blob/28db74f6f4692e97006ce9ebd8b19b9cc614b5b7/userspace/ksud/src/module.rs),
its [installer](https://github.com/tiann/KernelSU/blob/28db74f6f4692e97006ce9ebd8b19b9cc614b5b7/userspace/ksud/src/installer.sh),
[APatch's installer](https://github.com/bmax121/APatch/blob/e8e4651c16fb5075882a25813c8732285fdb3505/apd/assets/installer.sh),
and [Magisk's installer functions](https://github.com/topjohnwu/Magisk/blob/33bfff7ce750c27f157ddb4580e36bf842d6d95b/scripts/util_functions.sh).
All choose `modules_update` through `BOOTMODE` and update live module metadata
before the code swap. KernelSU also has an explicit hot-install dispatch and
effective metamodule installer selection. These source observations are inputs
to the pending adapter implementation, not device/fork qualification or proof
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
under renewed review in RFC0003. Common durable maintenance/manager/reboot
orchestration is still pending.

New GKI payload admission also checks the exact ELF's normal module-version
records: `module_layout` and every imported symbol must have a CRC. Trusted
signatures/hashes and declared init capabilities cannot substitute for those
records. The kernel loader still validates their values; this is not private ABI
proof. Historical source fencing deliberately accepts older catalogs so an
unqualified old generation can be retired safely rather than blocking recovery.
The [build qualification](../kmod/BUILDING.md#export-and-loader-version-qualification)
uses the same portable Rust inspector; generic non-MODVERSIONS test kernels have
a separate scope and cannot qualify a new GKI replacement.
