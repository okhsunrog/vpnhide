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

1. [Observation coordinator](../observation-coordinator.md): current cache runtime,
   dependencies, deadlines and limits.
2. [Config coordinator](../config-coordinator.md): implemented write lane and UI.
3. [Diagnostics state analysis](diagnostics-state-analysis.md): meanings of
   diagnostics, source history, semantic problems and acceptance questions.
4. [Transition contract](app-state-transitions.md): agreed machines and 30 scenario
   traces, especially diagnostic, presentation, capture and resource ownership.
5. [Diagnostics](../diagnostics.md), [debug bundle](../debug-bundle.md),
   [root mutation transport](../root-mutation-transport.md), plus scoped instructions.

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

1. **Operation impacts on diagnostics: done** (transition contract §19). The
   config coordinator publishes operation acceptance, mutating dispatches and
   results to `DiagnosticImpactObserver`; `reduceDiagnosticImpact` classifies
   relevance from the declared write set (own roles/hooks, global optional
   features, whole replacements), delays new runs on accepted relevant
   operations, interrupts a probing run at the first mutating dispatch, advances
   `changeEpoch`, and maps unresolved/failed operations into eligibility. The
   startup runtime reconcile is a forced activation without a write and is
   therefore not relevant: it neither delays nor interrupts the automatic suite
   (user decision, 2026-09-15). Next: surface applicability
   (`measurementApplicability`) and evidence sufficiency in a shared presentation
   projection instead of the legacy `State` (interrupted and operation-blocked
   attempts currently render as `Failed`). Preserve the feedback-cycle fixes above.
2. **Shared presentation: done** (transition contract §21):
   `DiagnosticsCache.presentation` combines the run view, routing observation,
   root snapshot, confirmed config and operation impact into one
   `DiagnosticPresentation` (eligibility, active run, latest attempt, latest
   complete measurement, applicability, evidence summary, `currentSuccess`),
   built by the pure `diagnosticPresentation`. Decided deviation: no global
   dispatcher; a `combine` of the coordinators' immutable states gives each
   emission one instant of all sources. The Diagnostics screen
   (`diagnosticScreenDecision`), the Dashboard hero (`heroDecision`), the bridge
   and the bundle (additive `diagnostics` object, no schema bump, golden refreshed)
   all render it; the legacy `State` remains only as the cached per-check report
   the tiles and the check list draw from. Device acceptance of the screen
   behaviour (VPN off/on, save during a run, interrupted run, manual recheck) is
   still owed under item 4. The plan that was executed, decided 2026-09-15
   (classify-then-render, per `lsposed/AGENTS.md`):
   - A pure `diagnosticBanner(presentation)` decision in `diagnostics/*Data.kt`
     with precedence: Initializing/Checking → progress; RestartApp → restart
     banner; VpnOff / SelfExcluded → the existing prompts; Applying,
     ApplicationUnknown, ApplicationFailed and routing Unknown → new explicit
     banners with a recheck action (no endless spinner, T17); then the run:
     active stage → progress; a non-completed latest attempt → its own banner
     (Interrupted: conditions changed during the check; Failed: the existing
     failed prompt) shown together with the last complete measurement; then
     applicability of that measurement: MatchesLastObservation → ready banner,
     Changed → "measured before <reason>, re-run", Unverified → "checking whether
     the results still apply"; Insufficient evidence → explicit, never green.
     The per-check list renders `measurementResults` (or active partial results)
     through `buildDiagnosticReport` as today.
   - Dashboard: hero from the presentation instead of `liveGate` overlaid on the
     cached `ProtectionCheck` (eligibility already includes routing currentness).
     `currentSuccess` plus no error issues → Protected; a measurement that is
     Changed/Unverified → Attention with a subtitle naming it; blocked eligibility
     → the existing prompts. Tiles keep coming from the cached report.
   - Bridge/bundle: additive `diagnostics` object on `VpnHideState` (eligibility,
     active run id, latest attempt outcome/failure, measurement run id and
     start/end, applicability, evidence counts and conclusion, currentSuccess);
     `gate`/`report` stay for compatibility; golden refresh, no schema bump
     unless a field changes meaning. `AgentControl.getState` fills it from
     `DiagnosticsCache.presentation.value`.
   - Strings EN/RU/ZH for the new banners; pure tests for the banner decision
     and the hero derivation; then a device pass over the VPN off/on, save
     during run and interrupted-run cases.
   Historical note on the previous state of this item: current source IDs reject obsolete cache computations,
   but screens still publish independently and retain last-good values. An atomic
   presentation revision including diagnostic measurement applicability is not
   implemented. Do not describe the cache stage as full cross-screen consistency.
3. **Capture through the run coordinator: done** (transition contract §20). The
   debug export no longer runs its own `runAllChecks`. `DiagnosticsCache.captureRun`
   requests an explicit run carrying a unique `captureId`, which is part of the
   request identity, so a capture never joins a suite whose probes began before its
   logging and counter baseline; the forensic order (acquire logging, clear dmesg,
   then run) is unchanged. The pure `debugSelfTestFrom` maps the terminal attempt
   onto the bundle's `gate` / `checkResults` / `selfTestRunId` / `errors` — only a
   completed run may be `ROUTED`, a blocked eligibility becomes its gate, and an
   interrupted, failed or never-admitted run contributes its evidence plus a reason.
   `exportDebug` returns `Written(file, errors)` or `Failed(reason)` instead of a
   bare `File?`, so an export can no longer vanish because `captureGateFrom` threw
   on unknown self-routing. Still using the older orchestration: capture
   reservation, cancellation and packaging as the agreed §9 capture machine, and
   `LogcatRecorder`.
4. **Final acceptance: wiring audit done, device pass owed.** The code was audited
   against §10 invariants and §11 traces on 2026-09-15 (independent read, then each
   finding re-verified against source). Fixed: an unresolved relevant operation was
   downgraded to `ApplicationFailed` when a queued operation settled `MutationPaused`
   behind it (I13; `unresolved` is now a set of operation ids cleared only by that
   operation's recovery); recovering an irrelevant operation (debug logging, another
   app) set `failed` for the self measurement; the explicit condition banners on the
   Diagnostics screen hid the last complete measurement (T17: history stays listed
   under Applying / ApplicationUnknown / ApplicationFailed / RoutingUnknown, the
   existing prompts still replace it); the Dashboard showed the "diagnostics failed"
   prompt for a run that never started because of a current condition the hero note
   already named (`HeroDecision.showsFailedPrompt`). Also fixed since that audit:
   (a) probe quarantine after a drain deadline was invisible to the user (admission
   `Rejected(ResourceUnavailable)` is dropped by `run`/`retry`, so the screen kept
   offering a Re-check that did nothing) — the presentation now carries
   `probeUnavailable` from `DiagnosticRunState.quarantined` and the Diagnostics
   banner, the Dashboard hero note and the bundle/bridge summary all say the
   diagnostic helper has not returned and no new check can start until it does;
   (b) relevance was classified from the submitted write set, so a `transform`-only
   write to `apps/<self>` (`writeStartupCanonical`, `mutateAgentApp`) was never a
   known change — `ConfigOperationObserver.prepared` now republishes the spec with
   the prepared write set merged in, before the operation's first mutating dispatch,
   and `DiagnosticImpactEvent.Prepared` turns that into the usual delay-then-interrupt
   (relevance only increases). Still open, with the decision recorded: (c) the
   Diagnostics list and the Dashboard tiles rebuild the report of a retained
   measurement against the *current* backend (§6 says coverage is never recalculated
   with a new backend); bounded because a backend change makes the measurement
   `Changed`, so the re-attributed list only ever sits under a "results changed"
   banner. (c) needs the backend stored in `MeasurementContext` and the report built
   from it, deferred. Latent, not
   reachable today: `DiagnosticRunCoordinator.handle()` for an evicted attempt id
   would return a deferred that never completes; `ensure` only passes the latest
   attempt id. Then validate on the device: root failures/timeouts, agent/UI
   conflicts, lifecycle changes during work, interrupted capture, plus the audit's
   device questions: whether `vhmutate` receipts prove descendant quiescence for a
   late success line; whether a VPN change between the Java probe window and the
   root sample is caught by the Verifying identity re-read (T15); how permanent a
   probe quarantine is in practice; the interrupt path for a save issued while a
   suite is probing; whether routing invalidation on every root refresh makes
   ResultsUnverified the steady-state banner. Host tests and successful launch are
   not full acceptance.

The batched root reader now waits for its launcher and pipe readers to finish.
The diagnostic run coordinator joins its own effect jobs when draining and
quarantines the probe resource on drain deadline, but the probe helpers
themselves (`GroundTruthProbe`, in-process JNI) are still blocking `su`/native
calls whose return is not proof of descendant quiescence. Coroutine
cancellation/return does not prove descendant quiescence. Do not mask this with a global root lock or make
config writes await diagnostics.

## Changelog fragments: consolidated

The branch accumulated one fragment per stage; on 2026-09-15 they were rewritten
relative to the last released version (1.2.5), not to the branch history: users
never saw intermediate states, so no fragment describes adding and then removing
something that only existed on this branch for hours (the RebootRequired sentence
was dropped for that reason). The four cold-start fragments became one entry, the
two configuration-switch fragments one, the two status-wording fragments one, and
the bundle/bridge and debug-log fragments one. Any later stage adds a fragment as
usual; re-check the set once more at release time.

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
Installed version: the build that removed RebootRequired (after `eeb01a47`),
user 0 only. The isolated device transport fixture
(`scripts/test-root-transport.py`) passed on it, including the `adopt` steps.
Decision 2026-09-15: no reboot is demanded for first adoption; see
[config coordinator](../config-coordinator.md) for the accepted risk. On 2026-09-15 the device had only user 0 (`pm list users`); profiles
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
| self-target preparation is the root gate (`252a225e`) | 1617, 1586, 1470 | no separate `su -c id`; preparation starts at ~90 ms |
| `vhmutate adopt`: inspect + open in one round trip (installed) | 1470, 1481, 1478 | config init 220 ms (staging 105 + adopt 110) instead of 320 |

Where the remaining ~1.5 s goes (run 2): helper staging `su` 105 ms; `adopt`
110 ms; the preparation root shell about 0.82 s (about 650 ms of section work
plus `su`/process overhead); routing probe 0.12 s; suite 0.14 s; derivation
0.05 s. The deferred reconcile then costs a background root snapshot (about
1.2 s, including package inventory) and a repeated derivation after paint.
Deferred by the user's decision: an activator "runtime unchanged" marker so the
reconcile skips its refresh; fewer `su` spawns inside the suite. Untouched:
folding helper staging into the adopt round trip (~100 ms).

Measurement caveat: with the screen dozing and the keyguard showing, the app
launches in the background scheduling group and every phase runs about three
times slower (`dashboard_ready` about 5 s). Check `dumpsys power` for
`mWakefulness=Awake` before trusting a trace.

Hardware checks completed: reproduced old theme-change navigation reset, then
verified Settings, a nested help article and selected Statistics tab survive
system theme changes and rotation on the fixed APK. Original system settings were
restored: night mode auto, accelerometer_rotation=0, user_rotation=0.
Cache race/quarantine scenarios were tested on the host, not exhaustively on-device.

When diagnostic logs are needed, obey `AGENTS.md`: if Debug logging is off, ask the
user to enable it; do not edit preferences through root or use UI to read diagnostic
results. Record logcat as a tracked background task, not with `adb logcat -d`.
