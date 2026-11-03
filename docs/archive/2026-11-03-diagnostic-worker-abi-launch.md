# Diagnostic worker ABI launcher source review — 3 November 2026

Historical source review at VPN Hide `1c0a3fee`; no device experiment was run.
This records a rejected launcher candidate, not a completed mixed-ABI worker.

The ordinary dedicated Service uses the package's selected ABI. Packaging both
JNI ABIs does not demonstrate a genuine second-ABI Zygote process. Loader image
proof must continue to distinguish the two ABIs.

`am instrument --abi armeabi-v7a` can request a process ABI, and manifest
`targetProcesses` can name the diagnostic process. However, the reviewed AOSP
`ActivityManagerService.startInstrumentation` stops the **target package** before
starting that process. This would disrupt the UI and its unsaved drafts.

- [Android 9 / API28 source](https://android.googlesource.com/platform/frameworks/base/+/refs/tags/android-9.0.0_r1/services/core/java/com/android/server/am/ActivityManagerService.java#22116):
  process selection precedes package-wide `forceStopPackageLocked`; the ABI
  override reaches `addAppLocked` afterwards.
- [Reviewed main source](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/am/ActivityManagerService.java#14560),
  blob `ce8dd66c10114d0cc73754e7c38ef9aede5277cb`, shows the same restart path.
  Its `noRestart` branch attaches instrumentation to an existing main process;
  it does not create a second-ABI diagnostic process.
- [Manifest contract](https://developer.android.com/guide/topics/manifest/instrumentation-element#trgproc):
  `targetProcesses` selects instrumentation recipients; it is not a promise of
  isolated package lifecycle.

No instrumentation entry point, package force-stop or relaxed image-proof gate
was added. A safe genuine secondary-ABI launcher remains implementation work;
Android version/vendor behavior and actual ABI-specific library loading require
device acceptance. Root `app_process` plus setuid is not a substitute. This
finding does not authorize a new helper APK/install flow or resume the paused
backend publication policy. Continue independent diagnostics implementation.
