# App state refactor: continuation handoff

Snapshot: 2026-09-15, implementation HEAD `a43cc3f0` before this document.

## Workspace and objective

- Work only in `/home/okhsunrog/code/vpnhide_state`, branch `refactor/app-state`.
- `/home/okhsunrog/code/vpnhide` has independent work. Do not switch, reset, merge
  into or modify that checkout. No push, PR or release is requested.
- Continue the agreed state architecture, next tackling diagnostic execution and
  measurement applicability. This is an implementation continuation, not a new
  architecture proposal. Speak Russian with the user.
- Follow root `AGENTS.md` and `lsposed/AGENTS.md`, including their required reading.
  Use small validated commits and EN/RU changelog fragments for visible changes.
  Do not add assistant attribution to commits, PRs or changelog entries.

## Read in this order

1. [Observation coordinator](observation-coordinator.md): current cache runtime,
   dependencies, deadlines and limits.
2. [Config coordinator](config-coordinator.md): implemented write lane and UI.
3. [Diagnostics state analysis](diagnostics-state-analysis.md): meanings of
   diagnostics, source history, semantic problems and acceptance questions.
4. [Transition contract](app-state-transitions.md): agreed machines and 30 scenario
   traces, especially diagnostic, presentation, capture and resource ownership.
5. [Diagnostics](diagnostics.md), [debug bundle](debug-bundle.md),
   [root mutation transport](root-mutation-transport.md), plus scoped instructions.

The documents record successive stages: statements such as "not connected yet"
in early implementation sections are historical. Check the current status and
source rather than treating every old remaining-work list as current. Likewise,
old device-validation paragraphs predate the successful installs described below.

## User decisions to preserve

- Switch position changes immediately with visible progress; persistence failure
  returns to confirmed state. Routine progress must not insert/remove layout rows.
- Rare blocking config problems use a dismissible dialog; ordinary known failures
  use a shared Snackbar. The user verified this UI and liked it.
- Drafts survive Activity recreation; persistence through process death is not
  required. UI field edits take priority over overlapping agent mutations. The
  agent receives explicit conflict information, including actual completed phases.
- Unknown root mutation outcomes get readback and one automatic repeat before
  pausing mutations. Never turn an unknown outcome into a claimed failure/success.
- Diagnostic meanings, validity and transitions require explicit reasoning using
  the historical investigation. Do not simply invalidate every result on every
  config change or equate no measurement with successful protection.
- Navigation must survive system day/night changes and rotation.
- Install VPN Hide only in the main Android profile, user 0. **Verify all profile
  installation flags after every install**, even when using `--user 0`.

## Implemented and checked

- Config transport, reducer, coordinator and all app write producers are connected.
  Main integration: `a183eec7`; layout-stable feedback: `e0d1ec21`.
- Observation runtime: `c71a060c`. `StateCache` delegates to process-owned
  `ObservationCoordinator`; older generations cannot publish success or failure.
  Waiter cancellation does not cancel shared work; timed-out workers remain
  quarantined until return. Root refresh invalidates declared dependents.
- App inventory publishes apps, profile names and warning as one packet. Agent
  reads share the cache. Root-derived projections carry internal source IDs.
- Diagnostic feedback-cycle repair: `c423c153`. Background Dashboard derivation
  observes terminal diagnostics rather than retrying Blocked/Failed. Explicit
  refresh retains retry behavior. `routedTransitions()` ignores temporary null
  readiness so a refresh cannot repeatedly retrigger failed diagnostics.
- Navigation restoration: `a43cc3f0`, using `rememberSaveable` for tabs, overlays,
  nested settings, search and filters. Editor drafts have separate ViewModel owners.
- Diagnostic execution runtime (transition contract §18): `DiagnosticsCache` is a
  facade over the process-owned `DiagnosticRunCoordinator` executing
  `reduceDiagnosticRun` with identified effects (`AppDiagnosticRunIo`). Runs are
  identified immutable attempts that survive screen changes; retry is a new run;
  a blocked eligibility is the `NotEligible` reducer event; measurement context
  (process/boot, self role and hooks, VPN interfaces and self routing, coverage)
  is captured at start and end; every check result has a stable id and the probe
  plan is derived from the four spec registries. Consumers still render the legacy
  `DiagnosticsCache.State` projection; `DiagnosticsCache.runs` exposes the view.
- Last verification: 671 JVM tests, warnings-as-errors compilation, ktlint, detekt,
  CPD and Android lintDebug passed for the diagnostic execution stage. The signed
  release build and signature verification were last run at `a43cc3f0`.

## Remaining implementation and next bounded step

1. **Operation impacts on diagnostics.** The run coordinator treats config
   readiness as settled and keeps `changeEpoch` at zero. Next: derive request
   dependencies from `CanonicalConfigRepository.state.operations`, observe their
   settlement (`OperationSettled`, including `Paused` → `ApplicationUnknown`),
   advance `changeEpoch` / send `ContextChanged` on a mutating dispatch with a
   relevant self projection, map `Applying`/`ApplicationUnknown` into eligibility,
   and make the startup automatic suite wait for the startup runtime reconcile
   instead of racing it. Then surface applicability (`measurementApplicability`)
   and evidence sufficiency in a shared presentation projection instead of the
   legacy `State` (interrupted attempts currently render as `Failed`).
   Preserve the feedback-cycle fixes above.
2. **Shared presentation.** Current source IDs reject obsolete cache computations,
   but screens still publish independently and retain last-good values. An atomic
   presentation revision including diagnostic measurement applicability is not
   implemented. Do not describe the cache stage as full cross-screen consistency.
3. **Capture ownership.** Logging tokens already use the config coordinator.
   Capture reservation, collection, cancellation, packaging and cleanup still use
   their older orchestration and need the agreed capture machine.
4. **Final acceptance.** Audit actual wiring against scenario traces, then validate
   root failures/timeouts, agent/UI conflicts, lifecycle changes during work and
   interrupted capture. Host tests and successful launch are not full acceptance.

The batched root reader now waits for its launcher and pipe readers to finish.
The diagnostic run coordinator joins its own effect jobs when draining and
quarantines the probe resource on drain deadline, but the probe helpers
themselves (`GroundTruthProbe`, in-process JNI) are still blocking `su`/native
calls whose return is not proof of descendant quiescence; the debug export still
runs `runAllChecks` outside the coordinator. Coroutine cancellation/return does
not prove descendant quiescence. Do not mask this with a global root lock or make
config writes await diagnostics.

## Build and device

Run from `/home/okhsunrog/code/vpnhide_state/lsposed`:

```sh
./gradlew --no-daemon --max-workers=2 -Dorg.gradle.jvmargs=-Xmx3072m -PvpnhideWarningsAsErrors :app:testDebugUnitTest :app:detekt cpdCheck :app:ktlintCheck :app:lintDebug
./gradlew --no-daemon --max-workers=2 -Dorg.gradle.jvmargs=-Xmx4096m -PvpnhideWarningsAsErrors :app:assembleRelease
```

Do not edit Kotlin/resources while Gradle runs: changing inputs during lint has
caused failures. Other Java processes may belong to unrelated work; do not kill them.
Signing is configured locally; never print keystore secrets. APK:
`/home/okhsunrog/code/vpnhide_state/lsposed/app/build/outputs/apk/release/app-release.apk`.
SDK: `/home/okhsunrog/Android/Sdk`; apksigner is in `build-tools/35.0.0`.

Device last used: Pixel 8 Pro, serial `3B241FDJG003LP`. Check availability before use.
Installed version: the deferred-reconcile build of this branch (after `68cc1c5c`),
user 0 only. On 2026-09-15 the device had only user 0 (`pm list users`); profiles
10 and 11 no longer existed. `adb install --user 0 -r` alone is not proof that
other profiles are clean. Inspect `pm list users` and `dumpsys package
dev.okhsunrog.vpnhide` after installation.

## Cold-start measurements (Pixel 8 Pro, VPN up, self routed)

Method: `adb logcat -c`, `am force-stop` + `am start -W`, then read the
`VpnHide-Startup` marks (Debug logging must be on). `dashboard_ready` in ms:

| Build | Runs | Notes |
|---|---|---|
| installed before this session (`1.2.5-139-g998ac6f7`, another branch) | 2980, 3059 | second root snapshot right after the first |
| `68cc1c5c` (diagnostic coordinator + Verifying reuse) | 4014, 3884, 3834 | startup reconcile invalidated root ~430 ms after the first snapshot; Verifying joined the reload |
| deferred reconcile (`a9900b13`) | 2470, 2329, 2472 | reconcile and its refresh run after first paint |
| whole-snapshot seed from the self-target read (`00b94862`) | 1717, 1702, 1710 | `root_snapshot_seeded`, no second shell before paint |
| self-target preparation is the root gate (installed) | 1617, 1586, 1470 | no separate `su -c id`; preparation starts at ~90 ms |

Where the remaining ~1.5 s goes (run 2): config coordinator initialization
320 ms (`vhmutate` inspect + open, two privileged round trips); the preparation
root shell about 0.8 s (about 650 ms of section work plus `su`/process overhead);
routing probe 0.12 s; suite 0.13 s; derivation 0.05 s. The deferred reconcile
then costs a background root snapshot (about 1.2 s, including package inventory)
and a repeated derivation after paint.
Next candidates: let the activator report "runtime unchanged" so the reconcile
skips its refresh (needs a redacted marker through the transport, like the
capacity warning); merge inspect+open into one `vhmutate` round trip; reduce `su`
spawns inside the suite (uid probe plus root `vhprobe`).

Hardware checks completed: reproduced old theme-change navigation reset, then
verified Settings, a nested help article and selected Statistics tab survive
system theme changes and rotation on the fixed APK. Original system settings were
restored: night mode auto, accelerometer_rotation=0, user_rotation=0.
Cache race/quarantine scenarios were tested on the host, not exhaustively on-device.

When diagnostic logs are needed, obey `AGENTS.md`: if Debug logging is off, ask the
user to enable it; do not edit preferences through root or use UI to read diagnostic
results. Record logcat as a tracked background task, not with `adb logcat -d`.
