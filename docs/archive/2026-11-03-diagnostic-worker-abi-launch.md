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

## Follow-up source review — 4 November 2026

The backend policy was subsequently accepted; the earlier pause above is
historical. The secondary32 launcher itself remains unimplemented. Two further
candidates were investigated without phone or provider operations:

- [Android14 ActivityManagerShellCommand](https://github.com/aosp-mirror/platform_frameworks_base/blob/android-14.0.0_r1/services/core/java/com/android/server/am/ActivityManagerShellCommand.java)
  provides the ABI override on instrumentation. The reviewed ordinary activity
  launch does not forward one. A proposed `am start --abi` activity therefore does
  not establish an independently selected 32-bit process.
- [ProcessList](https://github.com/aosp-mirror/platform_frameworks_base/blob/android-14.0.0_r1/services/core/java/com/android/server/am/ProcessList.java)
  has an internal ABI override, otherwise using the package's selected ABI. This
  is not a public per-Service manifest or ordinary start/bind override.
- [AppZygoteInit](https://github.com/aosp-mirror/platform_frameworks_base/blob/android-14.0.0_r1/core/java/com/android/internal/os/AppZygoteInit.java)
  preloads an APK through a LoadedApk class loader and optional ZygotePreload.
  That callback executes in the app zygote, not an ordinary app Service worker.
  The reviewed [ZygoteConnection](https://github.com/aosp-mirror/platform_frameworks_base/blob/android-14.0.0_r1/core/java/com/android/internal/os/ZygoteConnection.java)
  initializes a normal fork's entry point with a null class loader. Preloading
  APK classes does not by itself supply an AMS-managed second-ABI worker entry.

These observations reject simple launcher shortcuts; they do not prove that no
safe mechanism exists on any Android version. A root-created process, app-zygote
preload callback or manually loaded library must not be labeled ordinary Android
worker/provider coverage. Genuine Zygote provenance, app identity, lifecycle,
framework access, actual ABI and real provider loading all still need a concrete
implementation and acceptance. No extra helper APK, package force-stop, provider
refresh or weakened mixed-ABI qualification was introduced.
