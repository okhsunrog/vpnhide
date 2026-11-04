# Deferred secondary ABI launcher proposal — 4 November 2026

Historical draft only. Not approved for runtime integration, not compiled, and
never installed or run on a phone. The maintainer excluded this hook and launcher
from the current task on 4 November 2026 and requested a separate local branch.
They do not block qualification or delivery of the current work. Existing ABI
coverage limits must remain visible; no image/worker proof gate is relaxed.

Saved on local branch `research/secondary32-abi`, based on main
`b0999b7db7356cd2b7809bb762ab92f6b507b739`. The main checkout's tracked and
untracked work was not copied into this branch. The source drafts were prepared
against the earlier working tree based on `94793e118b1ff63a9b18aa56c4862bff34cc9a05`;
they need review and adaptation before any future implementation. In particular,
the archived Python sketch contains absolute paths and must not be executed.

## Origin and scope

The ROADMAP already listed genuine secondary32 worker launch as missing code.
The earlier [ABI launcher source review](../2026-11-03-diagnostic-worker-abi-launch.md)
explained why an ordinary Service uses the package-selected ABI and why
instrumentation was rejected: it may stop the target package and discard UI
drafts. An implementation continuation instruction then explicitly included the
secondary32 launcher among remaining tasks. This motivated investigation, but
there was no prior decision approving a system-server process-launch hook.

[RFC0005](../../rfcs/0005-unified-backend-lifecycle-policy.md) requires truthful
process/ABI coverage and forbids inferring full coverage from mapping. It does
not prescribe this hook. The concrete mechanism was a new proposal, not an
already approved design or a condition for shipping unrelated improvements.

## Proposed mechanism and limitations

An existing LSPosed module in system_server would intercept only the exact
six-parameter boolean overload of `ProcessList.startProcessLocked`:
`ProcessRecord, HostingRecord, int, boolean, boolean, String`.
On [reviewed AOSP Android14](https://github.com/aosp-mirror/platform_frameworks_base/blob/android-14.0.0_r1/services/core/java/com/android/server/am/ProcessList.java#L1592),
the final String is `abiOverride`. The draft would change null to `armeabi-v7a`
only for process `dev.okhsunrog.vpnhide:diagnostics32`, package
`dev.okhsunrog.vpnhide`, equal ordinary app UIDs in the primary Android user,
non-isolated launch, and a device advertising both ARM32 and ARM64. Existing
overrides are left alone. Missing or ambiguous signatures install no hook.

This entry point is shared: the callback would execute for other app launches,
even though its filter must not mutate their arguments. The draft does not yet
check HostingRecord or a one-time Core launch token. Catching Java exceptions
does not eliminate LSPosed or OEM platform risks, native crashes, shared-path
overhead, or an OEM retaining the signature while changing semantics.

AMS/Zygote would still create the ordinary Service process with Android-owned
UID, SELinux, framework access and lifecycle. Core would have to independently
verify genuine specialization, actual ARM32 ELF/image and current authority.
Hook installation and Service launch are not runtime or behavioral proof.

Alternatives considered were instrumentation with an ABI override (target-package
lifecycle disruption), root app_process (not an ordinary Android app worker), a
separate 32-bit APK (new install/distribution flow), or leaving secondary32
unverified. The maintainer selected deferral for the current task.

## Preserved files

The `.txt` suffixes deliberately keep these sketches outside every runtime and
test source set. They are reference material, not a patch to apply automatically:

- `SecondaryWorkerAbiHook.kt.txt` — uncompiled hook/filter sketch.
- `SecondaryWorkerAbiHookTest.kt.txt` — unexecuted draft identity-filter test.
- `wire_secondary.py.txt` — unexecuted integration sketch, including fixed Service
  and Rust loader proof routing; absolute paths are historical and unsafe to run.

No runtime hook or secondary Service is added by this branch.
