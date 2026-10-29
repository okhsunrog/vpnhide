# lsposed module — architecture & reuse rules

Scoped guidance for the Kotlin module (LSPosed hooks + Compose target-picker
app). Read before adding code here. The point of this file: stop the
duplication / god-function drift that AI-assisted edits cause when each change
only sees its local neighbourhood. **Reuse the abstractions below — don't
reinvent them.** `grep` for an existing helper before writing a new one.

## Where things live

The app package is no longer one flat directory. `hook/` is the load-bearing
split (next section); the rest group a feature's screen, its cache and its pure
model together:

| package | holds |
|---|---|
| `hook/` | everything LSPosed loads into `system_server` |
| `picker/` | the Hiding tab: app list, target model, per-app caches |
| `diagnostics/` | the check suite, the canonical report, the Diagnostics screen |
| `migration/` | pre-1.0 package/UID sources and strict 1.x/current JSON import, producing the current typed model |
| `settings/` | Settings and its sub-screens, plus the UI preference store |
| `debug/` | the bundle, the logcat recorder, kernel-image export |
| `statistics/`, `startup/` | the Statistics tab; MainActivity and startup wiring |
| `ui/`, `checks/`, `generated/` | design system, the JNI probe binding, codegen |

What stays in the root package is the shared vocabulary both sides use —
`CanonicalConfigData`, `StorageConfig`, `ShellUtils`, `RootSnapshotCache`, `HookMaskData`, `HookRegistry`,
`DashboardData`, the agent bridge. Moving those buys import churn and nothing
else; they belong to no single feature.

## "Diagnostic" names seven different things

The word got attached to every layer that answers "what is going on", so the
name alone will not tell you which one you are looking at. Nothing here is
misplaced — they genuinely are all diagnostics — but know which is which before
you add to any of them:

| name | what it actually is |
|---|---|
| `DiagnosticsScreen`, *Detailed diagnostics* | the user-facing check suite |
| `DiagnosticsCache`, `DiagnosticDomain`, `DiagnosticRunCoordinator` | the **run state** of that suite: identified, process-owned runs (`DiagnosticRunView`) folded with the observations into one `DiagnosticPresentation`, classified once into a `Situation` |
| `DiagnosticReport`, `buildDiagnosticReport`, `DiagnosticCheck` | the **canonical model** the screen and the bundle both render — see `docs/diagnostics.md` |
| `DiagnosticGate`, `RoutingGateCache`, `resolveDiagnosticGate` | the **precondition** for a meaningful run (VPN up, this app routed) — not a check |
| `HookDiagnostics`, `ConnectivityAttachDiagnostics`, `KpmDiagnostics` | attach/telemetry for the hooks themselves; **not part of the suite** |
| `writeDiagnosticZip`, `debug/` | the export — the artifact is the *debug bundle* (`docs/debug-bundle.md`) |
| `app_scan_diagnostics`, `kmod_diag` | section names inside that bundle |

The pair worth keeping straight: `DiagnosticsCache` holds a **run**,
`RoutingGateCache` holds a **precondition**, and only the second is derived from
the canonical config (so only the second is refreshed after a write).

## Two processes, one APK

The single most important thing about this module: `hook/` is loaded by LSPosed
**into `system_server`**; everything else runs in the app process. So `hook/`
carries no Compose, no Activity, no app resources — and nothing outside it may
touch `de.robv.android.xposed` (absent in the app process). The shared vocabulary
they both use (canonical-config parsing in `CanonicalConfigData`, mask vocabulary in `HookMaskData`, `LsposedStats`,
`LogTags`) stays in the root package. `HookPackageBoundaryTest` enforces both
directions, since `internal` is module-wide and the compiler will not.

`assets/xposed_init` names the entry class (`…vpnhide.hook.HookEntry`) and
`proguard-rules.pro` keeps it — moving or renaming it means editing both.

## Data flow

- **Read path:** one persistent process `CoreSession` publishes confirmed config,
  debug lease, native boot decision and per-main-app routing. Heavy batched core
  `RootSnapshot` probes supply inventory/counters/forensic details to typed snapshots
  (`DashboardState`, `TargetsSnapshot`) derived in pure functions → Compose.
  Root-derived projections carry the source observation ID and the accepted typed
  publication, joined identically after a fresh or seeded probe. Current native
  observation, desired config, delivery receipts and actual measurements are
  separate evidence. Dependent invalidation
  rejects obsolete loads; retained last-good screen values can temporarily belong
  to different observations while refreshes finish.
- **Write path:** field intents → process-owned `CanonicalConfigRepository` /
  `ConfigCoordinator` → one atomic typed field batch against fresh core state.
  Core owns the phase plan and durable recovery; the app has no phase executor or
  `CoreConfigIo`. Explicit import/bootstrap and SuperKey/cleanup use durable generated
  `CoreCommand` variants. The receipt carries persistence and native/Ports outcomes. Switches observe coordinator intent
  and confirmed values; Activity `CanonicalEditorViewModel` instances retain
  drafts and register conflicts. Observation refresh happens separately. LSPosed
  reads canonical JSON directly from `system_server`. See
  `docs/config-coordinator.md` for ownership, recovery and current migration limits.

## Load-bearing abstractions — reuse these

- **`StateCache<T>`** — base for app-scoped observation caches. It delegates to
  process-owned `ObservationCoordinator`; loading/error/value flows project one
  immutable observation. A new observation cache **extends this**; never hand-roll
  jobs or publish side metadata inside `load`. Root-derived caches declare
  `source = RootSnapshotCache.dependency`; app inventory uses `inventoryDependency`
  to avoid scanning icons after config-only changes. Use `ContextStateCache` when
  loading needs application context plus restart state. Readiness must use `current`,
  not retained `value`. See `docs/observation-coordinator.md` for lifecycle and
  publication rules. Diagnostic runs are a separate domain, not an observation cache.
- **`DiagnosticDomain` / `DiagnosticsCache`** — the one owner of the
  self-test suite. `DiagnosticDomain` is the domain wired from its inputs
  (observation flows, a `DiagnosticRunIo`, a scope, clocks), so
  `DiagnosticDomainTest` drives it exactly as production does with plain state
  flows and a fake helper; `DiagnosticsCache` is only the production wiring.
  Consume `DiagnosticSnapshot` when values must agree: it owns accepted inputs,
  presentation and Situation, and publishes before terminal handles complete.
  Bridge/export use its acceptance barrier; do not assemble independent flow values.
  It executes the pure `reduceDiagnosticRun` with identified effects on the
  process scope; a waiter detaching never cancels a run, and a retry is a new run.
  A suite starts in exactly three ways: `DiagnosticsCache.run` (startup intent:
  the first suite of the process, a join or a read afterwards),
  `retryDiagnosticsAndDashboard` (the one explicit entry point: Retry on both
  screens, pull-to-refresh, the post-reset re-check) and the owed confirmation
  (`owedConfirmation` in `DiagnosticConfirmationData.kt`: one automatic run per
  measurement key nothing covers). Never launch `runCoreChecks` from a screen,
  never request a run from a cache refresh hook, and never add an edge-triggered
  "the gate changed, run a suite" path — feed the observation and let the rule
  decide. Anything that must agree with the presentation (the Dashboard tiles
  and banners: `assembleDashboardState`) is projected from it, never cached
  beside it.
  The probe plan and per-run outcomes are keyed by the stable check ids in
  `NATIVE_CHECKS` / `NATIVE_EXTRA_CHECKS` / `CORE_JAVA_CHECKS` / `EXTRA_JAVA_CHECKS`
  — a new probe is a new spec entry with an id, not a bare list item.
  Config operations reach the suite only through `DiagnosticImpactObserver`
  (registered by `CanonicalConfigRepository`): relevance is decided by the pure
  `operationAffectsSelfMeasurement`, so a new kind of write must be classified
  there, never by adding a wait or a retry to a screen.
- **`RootSnapshotCache`** — the batched heavy root read (core's `RootSnapshot`
  probe). CoreSession owns current config, native selection and routing; do not
  derive a competing authority from a retained root snapshot. Need new detail on the Dashboard/Hiding path? Add a section to
  core's root snapshot (`crates/core-daemon/src/device_probes/root_snapshot.rs`);
  don't add another root read that races the snapshot. The app runs no `su`
  (CI: `tests/check_no_root_exec.sh`). The deliberate exception is
  `RoutingGateCache`: it consumes the session's per-main-app UID routing publication
  independently of the heavy root snapshot. There is no foreground polling timer.
- **`core/CoreRoot`** — bounded generated `CoreDiagnosticProbe` reads, logcat
  commands and debug leases over the shared core session. Privileged mutations
  use generated `CoreCommand` through the config coordinator. A new root read is
  a new core probe, never a shell line. Do not add serialized JSON control codecs:
  config, commands, publications, refusals and receipts cross UniFFI as generated
  records/enums; raw text belongs to import/export and forensic probe payloads.
- **`ShellUtils`** — startup self-target preparation and the parsers
  `parseConfigLines`, `parseKeyValueLines`, `parsePackageUidMap` (no shell
  despite the name). **Never write another `pm list`
  or `key=value` parser** — there used to be four; there is now one of each.
- **`ConfigChannels`** — the startup reconcile (core `RepairActivation`) and the
  auto-hidden-package refresh. Delivery to the native backend and Ports is core's
  job: it applies each saved config to the one backend it selected this boot.
  **Don't** hand-build per-backend runtime config in Kotlin; core derives each
  backend's wire from the canonical JSON.
- **`CanonicalConfigRepository`** — all app configuration intent, superkey and
  cleanup actions, and explicit repair go through its coordinator. Submit field
  intent or a pure transform; never save a cached full snapshot. Ordinary writes
  use typed Batch, full Replace is import/bootstrap only. Preserve core phase
  evidence. Unknown submission recovery queries the same core-issued op ID; compound commands
  must never be blindly replayed. See `docs/core-architecture.md`.
- **`CanonicalConfigData` / `StorageConfig`** — shared canonical JSON models/parsing,
  app-side config builders and migration helpers. The app never writes the file
  itself: core does, atomically, for each accepted edit. The canonical JSON is the persistent
  target config; legacy `targets.txt` / UID files are migration inputs or
  derived runtime state, not app-owned user config.
- **`TargetPickerScaffold`** — `TargetPickerScreen<T>`, `TargetRowShell`,
  `TargetChip`, `AppListScrollbar`. The unified Hiding list is built on these
  primitives; a new picker/list view should be too.
- **`StatusUi`** — `StatusColors` (pinned status palette — **never** use
  `MaterialTheme.colorScheme.errorContainer` etc. for status; Material You
  remixes them off-meaning), `StatusBanner`, `FileSaveShareRow`,
  `shareFileViaProvider`.
- **`NativeChecks`** — `NATIVE_CHECKS` is the single probe list (Dashboard
  summary + Diagnostics share it); `CheckStatus.toPassed()` is the single
  tri-state mapping.
- **`DashboardIssue` / `dashboardIssues`** — every dashboard banner is decided
  here, purely, from a `DashboardFacts`; `toMessage` (in
  `DashboardIssueRender.kt`) is the only half that words it. A new banner is a
  new `DashboardIssue` case plus its branch in the renderer — **never** a
  `res.getString` inside `loadDashboardState`, which is what made the guard list
  untestable for a year. Emission order in `dashboardIssues` is what the user
  sees; `DashboardIssuesTest` pins it.
- **`Situation` / `situation()`** (`diagnostics/SituationData.kt`) — what the
  user is looking at, classified once from the `DiagnosticPresentation` in one
  precedence; the Dashboard hero (`heroVisual`) and the Diagnostics banner
  (`diagnosticScreenDecision`) are wording maps over it and decide nothing.
  A new state is a new `Situation` case plus its branch in both maps — never a
  second precedence in a screen, and never a `res.getString` in the classifier.
  `SituationDataTest` pins the scenario table of
  `docs/notes/ui-state-presentation-review.md`.
- **`watchSystemDataDir`** — the shared `/data/system` FileObserver factory for
  the three system_server watchers (HookEntry / PackageVisibilityHooks /
  HookLog).
- **`VpnHideLog` / `HookLog`** — gated logging; don't `Log.*` directly on hot
  paths (stealth).

## Rules

- **Pure logic goes in top-level functions in `*Data.kt`, with a unit test** —
  not inside a composable or an orchestrator. `classifyKmodProblem`,
  `resolveLsposedState`, `buildNativeInstallRecommendation`, `dashboardIssues`
  are the pattern: data in, data out, no Android deps, tested. The recurring
  shape is classify-then-render — a pure function returning a decision, and a
  thin one turning it into strings. There is no Robolectric here, so anything
  that takes a `Context` or `Resources` is a function no test will ever cover:
  keep those as small as the wording itself. This is what keeps orchestrators
  (`loadDashboardState`) from rotting back into god-functions.
- **Keep functions short** (detekt fails new non-`@Composable` methods over
  ~60 lines). If an orchestrator grows, extract a pure helper.
- **Add a unit test** for any new pure function (JUnit, `src/test`).
- **`grep` before adding** any parser / formatter / status
  colour — it probably already exists above.
- **Changing a `@Serializable` type that reaches the debug bundle bumps the
  bundle schema.** `BundleSchemaGoldenTest` pins the serialized shape; when it
  fails, refresh the golden (`UPDATE_GOLDEN=1 ./gradlew :app:testDebugUnitTest
  --tests '*BundleSchemaGoldenTest*'`) and, if a field was removed/renamed or
  changed meaning, bump `VPNHIDE_STATE_SCHEMA` + add a row to
  [docs/debug-bundle.md §2.1](../docs/debug-bundle.md). Sealed subclasses that
  land in the bundle need an explicit `@SerialName` — without one kotlinx emits
  the fully-qualified class name, which changes the moment the class moves.

## Quality gates

- **ktlint** — formatting/style. Pre-commit hook (`.githooks/pre-commit`) + CI.
  Fix with `ktlint --format "lsposed/**/*.kt"`.
- **detekt** — complexity, function/file length, dead private members, bug
  patterns (the smell ktlint can't see). Config in `config/detekt/detekt.yml`.
  CI-enforced (`./gradlew :app:detekt`), runs clean with **no baseline**.
  When a new finding is genuinely inherent (a lookup table, an embedded shell
  script, priority-dispatch), opt out **at the call site** with
  `@Suppress("RuleName")` + a one-line reason — visible in the code, not hidden
  in a baseline. Only disable a rule in `detekt.yml` (with a comment) when it
  doesn't fit the codebase at all.
- **CPD** (copy-paste detector) — finds cross-file duplicated blocks that
  detekt can't (re-implemented parsers / save-builders are the classic
  AI-duplication smell). CI-enforced (`./gradlew cpdCheck`); report at
  `build/reports/cpd/`. Tune `minimumTokenCount` in the root `build.gradle.kts`
  if it ever false-positives.
