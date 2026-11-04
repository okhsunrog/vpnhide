# Loader catalog and recipient identities — 4 November 2026

A completed loader qualification/refactoring slice of
[RFC0003](../rfcs/0003-conditional-zygisk-publication.md) under the current
[RFC0005](../rfcs/0005-unified-backend-lifecycle-policy.md) policy. This is not the
managed projection staging executor or a new Zygisk activation path.

Core opens the authoritative catalog itself, independently of the recipient FD
provided by the companion. Observations retain separate source and recipient
device/inode identities. Directory FD leases pin those generations to prevent
inode reuse during their lifetime; repeated children share the recipient lease.
Source verification checks both exact raw manifest SHA and normalized metadata.
A peer-local catalog, swapped descriptor, replaced directory or whitespace-only
manifest replacement cannot reuse old qualification. One deadline includes
catalog, image and final source revalidation.

Availability and worker join require a live enabled recipient. Observations of
already-consumed process generations use the existing retained/revocation reader:
disabling future discovery or removing its library files does not remove hooks
from old processes. Such observations still require live process/image proof and
both established directory identities, and never imply full process coverage.

Production continues to pass the colocated compatibility recipient. No projection
copy, manager call, new module ID, warning grant, provider refresh or reboot is
introduced. Explicit Zygisk selection, managed projection staging, existing-user
continuity and migration offers remain implementation work. Genuine secondary32
Android bootstrap is still unimplemented; its availability gate is not weakened.

Final verification: 509 Core tests passed with no failures or ignored tests,
including twelve loader cases, real Zygisk crash/receipt recovery, process
ownership and root snapshot startup/contention. The four new loader regressions
cover independent source/recipient generations, peer-local catalog substitution,
exact manifest bytes and retained consumption after discovery withdrawal. Strict
host all-target Clippy with fixture-oracle and API28 ARM64/ARMv7 production
libraries/binaries passed, as did fmt, module-scaffold and APK no-root checks.
Public RPC33, journal16 and backend control2 are unchanged; no Kotlin/FFI/library
code changed, so the previous paired JNI/JVM qualification remains separate.

Logs and source hashes are retained outside the repository at
`/workspace/vpnhide-build-evidence/loader-source-recipient/`. Host descriptor and
process fixtures are not actual Android/provider/mixed-ABI acceptance. No phone,
manager installation, production signing or release operation occurred.
