# V1 improvement checklist

Work branch: `fix/v1-policy-cache`, based on `02bc7379ffd9240cc96a7b2b80b6c39e2170c3dd`.
The updated task is implemented in one branch. Only v1 is in scope.

## Implementation and review

- [x] **P1-01** — Enabled Java hook masks are unioned by appId at the Binder boundary. Disabled roles add no bits; every profile of a shared appId gets the same union. Package intent remains in canonical JSON. Inventory-detected sharing is explained in the Java dialog and all three offline-guide languages. Tests cover order, equal/different masks, disabled/empty/default roles, missing packages and profiles.
- [x] **P1-02** — Both canonical JSON and `packages.list` are fingerprinted before/after reads (device, inode, size, nanosecond mtime/ctime). Cache entries carry an atomic invalidation generation. Two attempts maximum; unstable reads retain a coherent prior value and retry on the next call. Tests drive file replacement, missed watcher events, invalidation during I/O and immediately before publication, and continuous changes. A watcher-thread test verifies invalidation does not wait for the reader lock.
- [x] **P2-03** — Typed read results distinguish errors from a successful empty policy. Errors retain last-good (empty at startup), publish an LSPosed metadata error and retry after one second without requiring a metadata change. Valid empty JSON or a removed canonical file clears policy. Recovery clears the error, including stat recovery without changed metadata. Tests cover these cache transitions; actual Android file-permission/SELinux scenarios remain device checks.
- [x] **P2-04** — Close-search and clear-query buttons have EN/RU/ZH accessible names. Resource compilation and lint pass; TalkBack focus/navigation remain device checks.
- [x] **P2-05** — Telemetry uses atomic same-directory rename only. Failure preserves the old complete file and in-memory counters, then schedules another flush. Tests verify a failed replacement/recovery and an open reader retaining the complete old inode across replacement. Actual target permissions/SELinux remain device checks.
- [x] **P3-06** — Screen subscription audit completed. Screen state collection uses lifecycle-aware APIs; the existing RESUMED-owned app-VPN poller and process-owned observation/config/diagnostic coordinators remain intact. Operation feedback subscriptions remain active to handle results of accepted work. Statistics-session polling intentionally continues in the background while the user exercises another app. Activity recreation/tab/background scenarios remain device checks.
- [x] **P3-07** — Import scans recurse through hook and app packages. Reviewed shared sources are checked for UI imports, unreviewed project imports and references to app-only declarations. Nested-source fixtures cover the recursive scan. Shared config parsing/model and hook-mask vocabulary are separated from app storage/report helpers. This is a source-level guard, not a separate Kotlin compile module or proof of reflective/third-party dependency isolation.
- [x] **P3-08** — Responsibilities reviewed. The boundary-driven extraction into `CanonicalConfigData` and `HookMaskData` has a concrete benefit; large screens and existing classifier tables were not split solely for line counts. Architecture guidance was updated in `lsposed/AGENTS.md`.
- [ ] **P3-09** — Device visual check pending: small display, landscape, large font, keyboard/bottom-sheet actions and reduced animations. No Android device was attached (`adb devices -l` returned an empty list). No density defect is claimed from source inspection.
- [x] **P3-10** — HTTP receives at most 32 KiB of request-head bytes, 64 headers, 8 KiB per line and 256 KiB of body, with one five-second monotonic deadline shared by head/body. Socket timeout follows the remaining budget. Receiving ends before operation dispatch, so this does not cancel an accepted root operation. Tests cover repeated/oversized headers, carriage-return inflation, slow reads, body deadline and a subsequent normal request.
- [x] **P2-11** — File exports derive Dashboard from the capture root snapshot and one saved diagnostic presentation, also used by the summary. Capture-owned gate, evidence and run ID remain unchanged. Dashboard assembly failure appends a section error and leaves the rest of the bundle available. Compilation, existing run/outcome tests and bundle golden validate contracts; full root capture remains a device check.
- [x] **P2-12** — The export sheet retains the full outcome and explains complete, partial and failed results in EN/RU/ZH. Partial archives keep Save/Share; failures retain their reason and retry path. Resource compilation/lint pass; interaction checks remain pending on device.
- [x] **P3-13** — Best-effort `beforeCapture` records available process state before logging/probes: timestamp, diagnostic summary, retained protection/messages, pending operation IDs and last phase/failure state. It performs no fresh root read and does not claim to be a previously viewed screen. It never blocks collection. Bundle golden covers the structure.
- [x] **P3-14** — Dashboard messages carry an explicit stable reason/parameter DTO and presentation language alongside existing text/severity/actions/help. Both file export and agent bridge consume the same Dashboard builder/classifier. Tests pin version/component parameters independent of translated text. Contract and golden are updated; additions remain within schema 2 under the existing compatibility policy.

## Verification

Run from the repository root with the configured Android/JDK/Rust environment:

```sh
ktlint 'lsposed/**/*.kt'
./lsposed/gradlew -p lsposed :app:testDebugUnitTest :app:detekt cpdCheck \
  :app:lintDebug :app:assembleDebug -PvpnhideWarningsAsErrors=true
```

All checks above passed; 844 JVM tests completed with zero failures or skips.

Runtime claims are limited to JVM tests and successful APK compilation/packaging.
No APK, TalkBack, LSPosed or root/SELinux scenario was executed on a device.
