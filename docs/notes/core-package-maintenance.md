# Authenticated Core replacement preparation

This describes implemented preparation, not a completed install/update flow.
RPC22, private journal11 and kernel control2/telemetry1 remain unchanged in this
cut. No device manager operation or boot write is performed by these APIs.

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

Still to implement: expose this preparation through the common durable
maintenance plan, retain its accepted floor and active work outside receipt
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
