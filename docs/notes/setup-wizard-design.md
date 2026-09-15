# Setup wizard — design (help-UX phase 4)

A design proposal for the first-run setup wizard — **not yet implemented**, to be
built in its own PR. The `file:line` references point at the tree as it was when
this was written and may drift; re-check them before implementing.

## 0. Ground truth the design rests on

Things verified by reading the code, because the design depends on them:

- **Root is a hard pre-gate, not a wizard step.** `VpnHideApp` renders
  `RootDeniedScreen` until `checkRootAccess()` succeeds and never composes
  `MainScreen` before that (`startup/MainActivity.kt:219-253`). Every wizard
  input (`DashboardState`) needs the root snapshot, so "grant root" can only be
  handled where it already is — the existing screen with its "Check again"
  button. The wizard starts at LSPosed.
- **Screens are flags, not a nav graph.** Settings and Diagnostics are
  early-return replacements (`MainActivity.kt:504-517`, `521-535`); Help is a
  `Box` overlay drawn *after* the `Scaffold` so tab state and the Hiding draft
  survive (`MainActivity.kt:568-818`), with `clearAndSetSemantics` on the
  scaffold while it is up (`:572`) and its own `BackHandler` inside
  `HelpScreen` (`help/HelpScreen.kt:114-129`).
- **The Hiding draft is `remember{}` and a teardown loses it.** MainScreen
  guards diagnostics/hidden-apps navigation from help with a confirm dialog when
  `protectionDirty` (`MainActivity.kt:560-566`). An overlay does not tear it
  down; only the early-return screens do.
- **Every signal the wizard needs is already derived once, purely.**
  `DashboardState` (`DashboardData.kt:328-349`) carries typed module states,
  `LsposedState`, `nativeInstallRecommendation`, `ProtectionCheck`; the issue
  list is decided in `dashboardIssues(facts)` (`DashboardIssues.kt:280-290`)
  and worded in `DashboardIssueRender.kt`. The live routing gate is
  `RoutingGateCache.gate` (`diagnostics/RoutingGateCache.kt:38`), overlaid onto
  the cached protection by the dashboard (`DashboardScreen.kt:85`, `:217-219`).
- **One gap: `DashboardState` exposes the rendered `messages`, not the typed
  issues.** `LsposedState.InstalledInactive` (`DashboardData.kt:106-109`)
  collapses four distinct situations — framework present but module never
  enabled, module disabled, enabled without System Framework, config
  unreadable — that `resolveLsposedState` folds together (`:947-981`); the
  distinction survives only as `DashboardIssue.LsposedNotEnabled` /
  `LsposedNoSystemScope` / `LsposedConfigUnreadable` (`DashboardIssues.kt:327-353`)
  and then as strings. Likewise `KpmAwaitingSuperkey`, `KpmStandaloneInstall`
  and the `ModuleBroken(problem)` text/artifact live only in the issue list.
  Section 3 resolves this.
- **`DashboardState` is serialized into the debug bundle**
  (`debug/VpnHideState.kt:113`), pinned by `BundleSchemaGoldenTest`. Any new
  field must be `@Transient` (with a default) or the golden is refreshed.
- **Persisted UI flags live in `SettingsRepository`** (DataStore
  `ui_settings`), with an established pattern for one-shot dismissals:
  `DONATE_PROMPT_DISMISSED`, `LEGACY_IMPORT_DISMISSED`, `SETTINGS_HINT_SEEN`
  (`settings/SettingsRepository.kt:41-44`, `settings/AppSettings.kt:53-73`,
  `settings/SettingsInteractor.kt:47-53`). Nothing outside `SettingsInteractor.kt`
  implements the interface, so adding a method touches two classes.
- **A Save refreshes the dashboard.** `CanonicalConfigRepository.derivedCaches`
  is `TargetsCache, DashboardCache, StatisticsCache, RoutingGateCache`
  (`CanonicalConfigRepository.kt:81-82`), so "pick a target" turns green on its
  own after Save. Nothing refreshes `DashboardCache` on ON_RESUME, though — the
  resume observer only touches the update check and the routing gate
  (`MainActivity.kt:471-485`). A user returning from LSPosed / the root manager
  needs an explicit refresh (§5).
- **Reusable pieces already exist:** `NativeInstallRecommendationCard`
  (`DashboardScreen.kt:1173-1312`) renders the exact recommended artifact,
  ambiguous-variant fallback, KPM-needs-runtime and Zygisk caveats, with a
  primary download button and a releases-page button; `ModuleDownloadButton` +
  `releaseAssetUrl` (`:1150-1170`); `VpnOffPrompt`, `SelfNotRoutedPrompt`
  (locale-gated accelerator link), `DiagnosticsFailedPrompt` (`VpnOffPrompt.kt`);
  `StatusBanner` / `StatusColors` (`StatusUi.kt:92`); `EnhancedCard` /
  `EnhancedButton` / `EnhancedOutlinedButton` (`ui/components/Enhanced.kt`);
  `HelpGuideEntry` row shape (`DashboardScreen.kt:359-388`); the dashboard's
  triple re-check (`RoutingGateCache.refresh` + `DashboardCache.refresh` +
  `DiagnosticsCache.retry`, `DashboardScreen.kt:242-246`). The download card and
  button are `private` today and need to become `internal`.
- **Help articles that map 1:1 onto steps:** `first-install`, `choosing-native`,
  `kmod-install` / `kpm-install` / `zygisk-install`, `ports`, `first-setup`,
  `configure-hiding`, `module-not-active`, `what-the-check-proves`,
  `game-accelerators` (en/zh only — `manifest.json` `locales`). Opened via
  `openHelp(articleId)` (`MainActivity.kt:537-540`); `vpnhide://` links come
  back through `handleHelpNav` (`:543-559`), scheme constant
  `IN_APP_SCHEME` in `help/MarkdownDoc.kt:21`.
- **First launch already pops two dialogs:** the changelog
  (`shouldShowChangelog` is true on a fresh install since `lastSeen == null`,
  `UpdateChecker.kt:215-219`, raised in `DashboardScreen.kt:114-140`) and the
  background-update prompt (`MainActivity.kt:233-250`). The wizard has to
  sequence against them (§6).
- **Quality gates** (`config/detekt/detekt.yml`): `LongMethod` 60 lines, exempt
  for `@Composable`; `CyclomaticComplexMethod` / `CognitiveComplexMethod` 20;
  `LongParameterList` 8; `LargeClass` 600; CPD `minimumTokenCount = 100` over
  hand-written Kotlin (`lsposed/build.gradle.kts:47-66`); lint
  `UnusedResources=error`. No Robolectric — anything taking `Context` /
  `Resources` is untestable; decisions go into pure `*Data.kt` functions
  (`lsposed/AGENTS.md` "Rules").

## 1. What the wizard is for (and is not)

For: a first-timer who will never read GitHub, on a device with root already
granted, gets an ordered checklist that tells them the *one* thing to do next,
detects when they did it (including across reboots), and ends with a verified
"the VPN is hidden" — without contradicting what the Dashboard says.

Not for: repairing a setup that used to work (the Dashboard issue cards own
that), choosing between backends by hand (`buildNativeInstallRecommendation`
owns that), or teaching the roles model (the guide owns that; the wizard links
to it).

## 2. Overall approach — options

### Option A — dedicated full-screen stepper overlay

A `SetupWizardScreen` composed exactly like `HelpScreen`: a `Box` overlay after
the `Scaffold` in `MainScreen`, gated by a `showSetup` flag, own `Scaffold` +
`TopAppBar` + `BackHandler`. Rows are step cards; the first not-done step is
expanded, done steps collapse to a green line, later steps are dimmed.

- Reuse: `NativeInstallRecommendationCard`, `ModuleDownloadButton`, the three
  gate prompts, `StatusBanner`, `EnhancedCard`. Step *status* derives from
  `DashboardState` + `RoutingGateCache.gate`, never recomputed.
- Cost: one new pure data file (~150 lines + tests), one new screen file
  (~350–450 lines split into 8–10 small composables), ~40 lines in
  `MainActivity`, ~15 lines of settings plumbing, ~40 strings × 3 locales.
- Fits the no-nav-lib constraint exactly (it is the fourth instance of the
  existing overlay pattern). Does not touch tab or picker state.
- Risk: a second surface that says things about modules. Mitigated by driving
  it from the same typed issue list the cards are rendered from (§3).

### Option B — "guided mode" on the Dashboard

When setup is incomplete, prepend an ordered checklist section to
`DashboardScreen` built by re-skinning the existing issue cards; hide the
severity-grouped banners while in that mode.

- Reuse: highest on paper (the cards *are* the messages).
- Problems found reading the code:
  - Issues only exist when something is wrong. A checklist needs *done* rows
    ("LSPosed active", "kmod active") and an *order*; `dashboardIssues` is
    ordered by severity-group emission (`DashboardIssues.kt:271-279`), not by
    setup order, and has no done concept. The re-skin would have to invent
    both, which is the same derivation Option A does — just inside the
    1473-line `DashboardScreen.kt`.
  - The Dashboard is the screen that overwhelms first-timers (hero, three
    module cards, recommendation card, update card, legacy banner, three
    message sections, donate, help entry). Adding a mode switch there grows
    the one file most at risk of becoming a god-composable again.
  - Two renderings of the same issues on one screen (checklist + the cards
    advanced users rely on) is the duplication/contradiction the brief warns
    about; hiding the cards in guided mode takes information away from the
    people diagnosing.
- Verdict: less new UI, more entanglement, worse for the target user.

### Option C — Option A + a compact Dashboard entry row (recommended)

Option A's overlay, plus a `SetupChecklistEntry` row on the Dashboard (same
shape as `HelpGuideEntry`, `DashboardScreen.kt:359-388`) reading
"Setup checklist — 2 of 4 done" while any mandatory step is pending. The row is
the re-entry point after a skip and the visible progress meter; the Dashboard
cards stay exactly as they are.

Why C over A alone: A needs some re-entry point anyway; a Settings entry is one
more hop away from where a first-timer looks. Why C over B: everything in the
B verdict. **Recommendation: C.**

## 3. Step model and completion detection

### 3.1 Feed the wizard the typed issue list

Add to `DashboardState` (`DashboardData.kt:328`):

```kotlin
@Transient val issues: List<DashboardIssue> = emptyList(),
```

populated in `DashboardFacts.toDashboardState` (`:1515-1538`) from the same
`dashboardIssues(facts)` call that produces `messages` (`:1609`). `@Transient`
with a default keeps the bundle golden untouched (kotlinx skips it) and keeps
`DashboardIssue` free of `@Serializable`.

This is the single decision that makes "never contradicts the cards"
structural: the wizard's per-step status is a pure function of the same list
the cards are rendered from, and a new issue case that matters to setup is one
`when` branch away.

Alternative considered: adding a `reason` enum to
`LsposedState.InstalledInactive`. It fixes only the LSPosed step (KPM
superkey / standalone / broken-module cases still need the issues), touches a
serialized type (golden refresh + a `docs/debug-bundle.md` row), and creates a
second source for a fact the issue list already states. Rejected.

### 3.2 Pure model (`startup/SetupWizardData.kt`)

```kotlin
internal enum class SetupStep { JavaLayer, NativeBackend, Ports, FirstTarget, Verify }

internal sealed interface StepStatus {
    data object Done : StepStatus
    data object Todo : StepStatus                       // nothing installed / configured yet
    data object WaitingReboot : StepStatus              // installed, loads on next boot
    data object NeedsAppRestart : StepStatus            // Verify only: DiagnosticGate.NEEDS_RESTART
    data class Blocked(val issue: DashboardIssue) : StepStatus  // a named problem; render its text/artifact
    data object Unverified : StepStatus                 // runtimeCheckable == false
    data object Skipped : StepStatus                    // Ports only (optional, nothing installed)
    data class Gated(val gate: DiagnosticGate) : StepStatus     // Verify only: VPN_OFF / SELF_NOT_ROUTED
}

internal data class SetupChecklist(
    val statuses: Map<SetupStep, StepStatus>,
    val current: SetupStep?,       // first mandatory step not Done; null when complete
    val doneCount: Int, val mandatoryCount: Int,
) { val complete get() = current == null }

internal fun deriveSetupChecklist(
    state: DashboardState,
    liveGate: DiagnosticGate?,     // RoutingGateCache.gate — overlays state.protection like the dashboard does
): SetupChecklist
```

Each step's derivation is its own private function (keeps every function well
under 60 lines and the `when`s small):

| Step | Done | WaitingReboot | Blocked / other | Todo | Signals (all on `DashboardState`) |
|---|---|---|---|---|---|
| **JavaLayer** | `lsposed is LsposedState.Active` | `lsposed is LsposedState.NeedsReboot` | `issues` has `LsposedConfigUnreadable` → Blocked(issue) | `NotInstalled` → "install LSPosed"; `InstalledInactive` → sub-text from `issues`: `LsposedNotEnabled` ("enable VPN Hide, add System Framework") or `LsposedNoSystemScope` ("add System Framework") | `lsposed`, `issues` |
| **NativeBackend** | `moduleActive(nativeBackend.state)` (covers Builtin: `nativeBackend.id == Builtin` is just "detected, nothing to install") | `(nativeBackend.state as? Installed)?.pendingReboot == true`, or Installed && !active && no problem issue && `runtimeCheckable` (installed by hand, not staged) | `issues` has `ModuleBroken` for a native kind, `KpmAwaitingSuperkey`, `KpmStandaloneInstall`, `NativeConflictKernel`, `NativeConflictDeferred` → Blocked(first such issue, in `dashboardIssues` emission order); `runtimeCheckable == false` → Unverified | `nativeInstallRecommendation != null` (only non-null when nothing is installed — `DashboardData.kt:1531-1532`) | `nativeBackend`, `nativeInstallRecommendation`, `issues` |
| **Ports** (optional) | `ports is Installed` (active or not — the rules apply on Save, `ports.md`) | — (ports needs no reboot) | `issues` has `PortsRulesInactive` after a target exists → shown as a note, never blocks | `NotInstalled` → Skipped (row offers the zip + Learn more) | `ports`, `issues` |
| **FirstTarget** | `issues` has no `NoTargets` (that issue is exactly `targets.lsposed + targets.native == 0`, `DashboardIssues.kt:395`) | — | — | `NoTargets` present | `issues` |
| **Verify** | `protectionFullyPassed(protection)` (`DashboardData.kt:353-356`) with effective protection = `liveGate?.blockedOrNull()?.let { Blocked(it) } ?: state.protection` (same overlay as `DashboardScreen.kt:217-219`) | — | `Blocked(NEEDS_RESTART)` → NeedsAppRestart; `Blocked(VPN_OFF)` / `Blocked(SELF_NOT_ROUTED)` → Gated(gate); `ProtectionCheck.Failed` → Blocked(ChecksFailed-like retry state); `Checked` with leaks → Blocked(`ChecksFailed`) → "Details" opens Diagnostics | Not reachable until JavaLayer, NativeBackend, FirstTarget are Done — before that the row is dimmed "after the steps above" | `protection`, `liveGate` |

Notes on the table:

- `current` = first of `JavaLayer, NativeBackend, FirstTarget, Verify` whose
  status is not `Done`. Ports never participates. `mandatoryCount = 4`.
- **`SELF_NOT_ROUTED` is not a failure.** The row reads "Can't self-test on
  this VPN — expected with game accelerators and per-app VPNs" with the same
  copy as `self_not_routed_prompt` (`strings.xml:193`), a Retry, the
  locale-gated accelerators link, and a **Finish anyway** that marks the wizard
  done. The checklist is `complete` for auto-hide purposes when the only
  non-Done mandatory step is `Verify` in `Gated(SELF_NOT_ROUTED)`; the flag
  from §4 records the user's choice.
- **`NEEDS_RESTART` is the state a first-timer *will* hit on the very first
  launch**: `ensureSelfInTargets` adds the app to its own targets and reports
  `selfNeedsRestart` (`ShellUtils.kt:254-260`), which parks
  `DiagnosticsCache` at `Blocked(NEEDS_RESTART)` for the whole process
  (`diagnostics/DiagnosticsCache.kt:107-115`). In practice the module-install
  reboots clear it, but the row must still say "Restart VPN Hide (force-stop
  and reopen)" — reuse `dashboard_needs_restart` (`strings.xml:191`).
- **Why derived, not a stored step index.** A stored index would be wrong
  after: a reboot (the app is killed — the index survives but the world
  changed), a module installed from the root manager while the app was closed,
  LSPosed scope edited in its own app, a failed insmod (index says "rebooted",
  reality says "broken"), or the user uninstalling a module. Deriving from
  `DashboardState` gives the correct step on every launch for free, and every
  "did it work?" answer is the same one the Dashboard would give. The only
  thing worth persisting is the user's *intent* (skip / finish) — §4.

### 3.3 Ownership: what the wizard must never do

- No `suExec` of its own; no re-read of the LSPosed DB; no kernel parsing. If a
  signal is missing, it is added to the root snapshot / `DashboardFacts` /
  `DashboardIssue` and the wizard consumes it.
- No re-deriving the artifact name: it comes from
  `nativeInstallRecommendation.recommendedArtifact` (Todo) or
  `DashboardIssue.ModuleBroken.problem.downloadArtifact` (Blocked).
- No second wording for a known issue: a `Blocked(issue)` row renders
  `issue.toMessage(context, res).text` and reuses `messageActionSlot`
  (`DashboardScreen.kt:426-444`) for its button — the same download / Details /
  Learn more precedence as the cards. Both need to become `internal`.

## 4. Resumability and reboot handling

- **Landing on the right step after a reboot:** nothing to do. The app cold
  starts, `DashboardCache` loads, `deriveSetupChecklist` yields the first
  not-Done step, and the auto-open rule (§5) shows it. The expanded step is
  `checklist.current`; the user can tap any row to expand it, but that
  expansion is `remember{}` in the overlay and is intentionally lost on close.
- **"Waiting for reboot" copy** (LSPosed `NeedsReboot`, native `pendingReboot`):
  amber `StatusBanner` "Installed — reboot to activate", a one-line
  "After the reboot, open VPN Hide again; this checklist picks up where you
  left off." No reboot button in v1 (§7).
- **Returning from LSPosed / the root manager without a reboot** (scope
  edited, module flashed): `DashboardCache` is stale. Two cheap fixes,
  recommend both: a **Check again** button at the top of the overlay (calls
  `startupCoordinator.refreshDashboard(scope, refreshRestart)` — the same call
  the Dashboard refresh button makes, `MainActivity.kt:636-638`), and an
  ON_RESUME refresh while `showSetup` is true, throttled with a `remember`ed
  timestamp (mirror `RoutingGateCache.refreshIfStale`'s 4 s idea; the existing
  `LifecycleEventObserver` at `MainActivity.kt:471-485` is the hook). The
  refresh runs a full root shell (~1–2 s) — acceptable only while the wizard is
  up, which is why it is gated on `showSetup`.
- **Persisted state — exactly one flag:** `AppSettings.setupWizardDone`
  (DataStore key `setup_wizard_done`, setter on `SettingsRepository`,
  `SettingsInteractor.setSetupWizardDone`). Set by **Skip** ("I'll do it
  myself") and by **Finish** on the completion card / Finish anyway. It is
  never read to decide step status — only whether to auto-open.

## 5. Entry and exit

- **Auto-open**, evaluated once per process in `MainScreen` after
  `dashboardState` is first non-null:
  `settingsLoaded && !settings.setupWizardDone && !checklist.complete`. Once
  per process so a mid-session refresh cannot pop the overlay over the picker.
- **Never auto-open again after `setupWizardDone`.** If a working setup later
  breaks, that is the Dashboard's job (issue cards). The wizard is first-run
  only.
- **Re-entry:** the `SetupChecklistEntry` row on the Dashboard, rendered while
  `!checklist.complete` (regardless of the flag) — it disappears on its own
  when everything is green, and reappears if, say, the user removes every
  target. Placed directly above `HelpGuideEntry` (`DashboardScreen.kt:349`).
  No Settings entry in v1.
- **Skip / close:** the top bar has a back arrow (close, flag untouched — the
  row remains) and an overflow-free text button **Skip** that sets the flag and
  closes. `BackHandler` closes.
- **Help on top of the wizard:** "Learn more" calls `openHelp(article)`; since
  `HelpScreen` is drawn after the wizard in the same `Box`, it stacks above and
  closing it returns to the wizard with its expansion intact. A `vpnhide://`
  link inside an article goes through `handleHelpNav`, which must also set
  `showSetup = false` — otherwise "Open the Hiding tab" would land under the
  overlay.
- **Coexistence with the Dashboard:** cards untouched; the wizard adds no
  banner to the Dashboard. The recommendation card
  (`NativeInstallRecommendationCard`) stays on the Dashboard *and* is reused
  inside the native step — same composable, same data, no drift.
- **Coexistence with first-launch dialogs:** gate the background-update prompt
  (`MainActivity.kt:234-237`) and the changelog dialog
  (`DashboardScreen.kt:114-133`) on `!showSetup` so a first-timer sees exactly
  one thing. Both fire naturally on the next launch or after Skip/Finish. This
  is two one-line conditions; the changelog one needs `showSetup` (or a
  `LocalSetupWizardVisible`) passed down — simplest is a `Boolean` parameter on
  `DashboardScreen`.
- **Semantics:** extend the existing `clearAndSetSemantics` condition
  (`MainActivity.kt:572`) to `showHelp || showSetup`.

## 6. Per-step UI

Layout: `Scaffold` + `TopAppBar` ("Setup", back arrow, **Skip** text action,
**Check again** icon action showing `ButtonSpinner` while `dashboardLoading`),
a progress line ("2 of 4 done", `LinearProgressIndicator`), then a
`LazyColumn` of `SetupStepCard`s. Each card: leading status glyph
(check / amber dot / red dot / grey number), title, one-line status; the
`current` card is expanded with body + actions. Colours from `StatusColors`
only.

| Step | Expanded body | Primary action | Secondary | Learn more |
|---|---|---|---|---|
| Java layer | Todo/NotInstalled: "Install LSPosed (or LSPosed-Next / Vector), then come back." Todo/NotEnabled: "In LSPosed, enable **VPN Hide** and add **System Framework** to its scope." NoSystemScope: the scope sentence only. WaitingReboot: reboot banner. Blocked: issue text. | — (no reliable LSPosed manager intent; its package is often randomised/hidden) | Check again | `first-install` |
| Native backend | Todo: `NativeInstallRecommendationCard(recommendation)` verbatim — it already words kmod / ambiguous-variant / KPM-needs-runtime / Zygisk and carries the download + all-releases buttons — plus one wizard sentence: "Install the ZIP from your root manager's Modules screen, then reboot." WaitingReboot: reboot banner. Blocked: `issue.toMessage().text` + `messageActionSlot` (download the right zip / Learn more). Unverified: neutral "can't verify from this shell" note. | Download (inside the card) | Check again | by recommendation: `kmod-install` / `kpm-install` / `zygisk-install`; Blocked → `module-not-active` |
| Ports (optional) | "Only needed if your VPN/proxy runs a local port (Clash, sing-box…). Skip if unsure." Done: "Installed". | `ModuleDownloadButton("vpnhide-ports.zip")` | — | `ports` |
| First target | "Pick the app you want to hide the VPN **from** (a bank, not the VPN client), turn on Java and Native, Save." Done: "N apps configured" (from `nativeTargetCount` / `LsposedState.Active.targetCount` when available, else just "Configured"). | **Open Hiding** → close wizard, `currentTab = Protection` | — | `first-setup` |
| Verify | Gated(VPN_OFF): `VpnOffPrompt(onRetry)`. Gated(SELF_NOT_ROUTED): `SelfNotRoutedPrompt(onRetry, onOpenAccelerators)` + **Finish anyway**. NeedsAppRestart: `dashboard_needs_restart` banner. Failed: `DiagnosticsFailedPrompt`. Checked+leaks: "Some checks still see the VPN" + **Details** (Diagnostics). Done: green "VPN hidden — every check passed." | Retry = the dashboard's triple re-check, extracted to one shared function (§8) | Details → `showDiagnostics = true` | `what-the-check-proves`; accelerators link only for en/zh (same gate as `SelfNotRoutedPrompt`) |
| Completion card | Shown when `checklist.complete`: "Setup complete" + "Force-stop and reopen the app you hid the VPN from so it checks again" (the step people forget, `first-setup.md` step 6). | **Finish** (sets flag, closes) | — | `configure-hiding` |

Rows below `current` are dimmed, not disabled — tapping expands them
read-only so a curious user can see what is coming.

## 7. v1 versus deferred

**v1 (ships as one PR, one changelog `added` entry):**

- Overlay + Dashboard entry row (Option C); four mandatory steps + optional
  Ports row; statuses derived as in §3; `@Transient issues` on
  `DashboardState`; `setupWizardDone` flag; auto-open once per process;
  Skip / Finish / Finish anyway; Check again + resume refresh while open;
  Learn more per step; Open Hiding; Details; reuse of the recommendation card,
  download button, gate prompts, shared re-check; dialogs gated on
  `!showSetup`; strings EN/RU/ZH; unit tests for the derivation.

**Deferred (explicitly not in v1):**

- A **Reboot now** button (`suExec("svc power reboot")` behind a confirm
  dialog). Cheap and the single most useful follow-up, but it is a new
  privileged side effect and interacts with the Hiding draft; do it as v1.1
  once the checklist copy has settled.
- Deep-linking into LSPosed / Magisk / KernelSU / APatch. Package names are
  unreliable (LSPosed's manager is a randomised/hidden package, Magisk is
  commonly repackaged); a dead button is worse than none.
- Embedding the target picker inside the wizard, or auto-suggesting a target
  (e.g. detect installed banking apps). The Hiding tab is one tap away and its
  draft state is `remember{}` — reusing it in a second host is a refactor of
  its own.
- Inline SuperKey entry for the KPM path. The Blocked row already names
  Settings → Security; adding a `SettingsSubScreen.Security` deep link is a
  small follow-up if support asks show it is needed.
- Per-step screenshots (`HelpImage` exists; the articles already carry them),
  animations beyond the standard fade, confetti, progress persistence for
  analytics.
- Re-triggering the wizard when a working setup regresses.

## 8. Implementation outline

Phase 1 — data (pure, tested; no UI):

1. `DashboardData.kt`: `@Transient val issues: List<DashboardIssue> = emptyList()`
   on `DashboardState`; pass `dashboardIssues(facts)` through
   `toDashboardState` (compute once, map to `messages` from the same list).
   Run `BundleSchemaGoldenTest` to confirm no golden change.
2. New `startup/SetupWizardData.kt`: `SetupStep`, `StepStatus`,
   `SetupChecklist`, `deriveSetupChecklist(state, liveGate)`, private per-step
   helpers `javaLayerStatus`, `nativeStatus`, `portsStatus`, `targetStatus`,
   `verifyStatus`, plus `shouldAutoOpenSetupWizard(settingsLoaded, done, checklist)`.
   Each helper < 25 lines; the `when`s stay under the complexity thresholds
   because the branching is split per step.
3. New `test/.../SetupWizardDataTest.kt` using the `dashboardState(...)`
   fixture style of `DashboardUiStateTest`: fresh device (all Todo, current =
   JavaLayer); LSPosed NeedsReboot → WaitingReboot; InstalledInactive +
   `LsposedNoSystemScope` → Todo with that issue; kmod `pendingReboot` →
   WaitingReboot; kmod `ModuleBroken` with artifact → Blocked carrying the
   artifact; `KpmAwaitingSuperkey` → Blocked; `nativeInstallRecommendation`
   non-null → Todo; Builtin active → Done; `runtimeCheckable=false` →
   Unverified; ports NotInstalled → Skipped and never in `current`; `NoTargets`
   → FirstTarget Todo; gate matrix for Verify (NEEDS_RESTART / VPN_OFF /
   SELF_NOT_ROUTED / ROUTED+ok / ROUTED+leaks / Failed); `complete` semantics
   incl. the SELF_NOT_ROUTED "finish anyway" case; auto-open gating.
4. Settings: `AppSettings.setupWizardDone`, `Keys.SETUP_WIZARD_DONE`,
   `SettingsRepository.setSetupWizardDone`, `SettingsInteractor` +
   `RepositorySettingsInteractor` methods; extend `SettingsDataTest` if it
   round-trips defaults.

Phase 2 — reuse surface (no behaviour change):

5. `DashboardScreen.kt`: `ModuleDownloadButton`, `NativeInstallRecommendationCard`,
   `messageActionSlot`, `LearnMoreButton`, `DetailsButton` → `internal`
   (or move the download pieces to `ui/components/ReleaseDownload.kt` to keep
   `DashboardScreen.kt` from growing).
6. Extract the dashboard's re-check triple (`DashboardScreen.kt:242-246`) into
   `internal fun recheckProtection(scope, context, selfNeedsRestart)` (natural
   home: `StartupCoordinator`, next to `refreshDashboard`) and call it from
   both the Dashboard and the wizard — CPD would otherwise flag the clone.

Phase 3 — UI:

7. New `startup/SetupWizardScreen.kt` (all `@Composable`, each small):
   `SetupWizardScreen(checklist, loading, callbacks, onClose)`;
   `SetupWizardTopBar`; `SetupProgressLine`; `SetupStepCard(step, status, expanded, onToggle, content)`;
   `JavaLayerStepBody`, `NativeStepBody`, `PortsStepBody`, `TargetStepBody`,
   `VerifyStepBody`; `RebootBanner`; `SetupCompleteCard`. Callbacks bundled in
   one `SetupWizardActions` data class (open Hiding, open Diagnostics, open
   help(article), check again, retry, skip, finish) to stay under
   `LongParameterList`.
8. `DashboardScreen.kt`: `SetupChecklistEntry(checklist, onClick)` above
   `HelpGuideEntry`; `showSetup: Boolean` parameter gating the changelog dialog.
9. `MainActivity.kt`: `showSetup` flag; `deriveSetupChecklist` from
   `dashboardState` + `RoutingGateCache.gate` (`remember(state, gate)`);
   once-per-process auto-open `LaunchedEffect`; overlay after the `Scaffold`,
   before `HelpScreen`; `clearAndSetSemantics` on `showHelp || showSetup`;
   `handleHelpNav` closes the wizard; resume refresh while shown; background
   update prompt gated on `!showSetup`.

Phase 4 — strings and docs:

10. Strings (EN, RU idiomatic — no calques, ZH): ~40 keys under a
    `setup_` prefix: title, skip, finish, finish_anyway, check_again, progress
    ("%1$d of %2$d done"), five step titles, per-status one-liners
    (todo/waiting_reboot/done/blocked/unverified/skipped/gated), the LSPosed
    three instructions, the native "install from Modules then reboot" sentence,
    ports blurb, target blurb + done count (plural), verify leak sentence,
    completion title/body, Dashboard entry label. Reuse existing keys where the
    text is the same (`dashboard_needs_restart`, `vpn_off_prompt`,
    `self_not_routed_prompt`, `dashboard_install_recommendation_download`,
    `dashboard_action_learn_more`, `dashboard_action_details`,
    `vpn_off_retry`). Every new key is referenced (lint `UnusedResources`).
11. `docs/help/*/first-install.md`: one sentence pointing at the in-app
    checklist ("the Dashboard's Setup checklist walks you through these
    steps"); `changelog.d` `added` fragment EN/RU.

Testability summary: every decision (status per step, current step, complete,
auto-open) is in `SetupWizardData.kt` with no Android imports and is covered by
JUnit; the composables only render a `SetupChecklist` and forward callbacks.
Gates: no new non-Composable function approaches 60 lines; the re-check
extraction and `internal` reuse prevent CPD clones; `LargeClass` is not
approached (`SetupWizardScreen.kt` is top-level functions); every string is
used; `BundleSchemaGoldenTest` stays green because the only serialized type
touched gains a `@Transient` field.

## 9. Open questions for the maintainer

1. Should **Skip** be prominent (text button in the top bar) or tucked away
   (bottom link)? The design assumes prominent — a power user should not have
   to hunt for it — but it makes first-timers more likely to dismiss.
2. Is `SELF_NOT_ROUTED` → **Finish anyway** acceptable, or should the wizard
   insist on a routed self-test? Accelerator users (a stated audience) can
   never satisfy the routed gate, so insisting would leave them stuck on the
   last step forever.
3. Ports as an optional row in the checklist vs. not mentioning it at all in
   the wizard (leave it to `configure-hiding`). The row is cheap; the question
   is whether it distracts.
