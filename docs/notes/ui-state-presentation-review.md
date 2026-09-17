# UI state presentation: design review

Status: review of branch `refactor/app-state-cleanup` (PR #338) as of commit
`4f6209e7`, 2026-09-16, written read-only. Approach C (§3, §4) was then
implemented on branch `refactor/ui-situation` the same day; the record of what
runs is transition contract §22. Decisions on the open questions (§4): 1 re-run
on explicit; 2 one confirmation suite on a routed transition while the Dashboard
is up; 3 Transition grace 0; 4 neutral grey, no fifth colour; 5 self excluded
neutral; 6 own-roles save keeps Changed plus a manual re-check (earlier
maintainer decision); 7 foreground return is Background; 8 additive bundle
fields, no bump. Stage 8 (activator "runtime unchanged" marker) was declined in
favour of a targeted invalidation if the background work ever matters. Paths
below are relative to `lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/` and
describe the code as it was before the change.

Scope: the Dashboard hero (title, colour, subtitle), the prompts under it, the
skeleton gate, and the Diagnostics banners, all of which render
`DiagnosticPresentation`. The question is whether the decision layer between the
presentation and those surfaces is the right model, and what to do about the
transitions seen on the device today.

## 1. Diagnosis

### What the current layer gets right

- One projection for every consumer (`DiagnosticsCache.presentation`,
  `DiagnosticsCache.kt:89-121`): eligibility, run, attempt, measurement and
  applicability come from one instant. This is the property everything below
  builds on; keep it.
- Classify-then-word is real: `heroDecision` (`DashboardHeroData.kt:66-77`) and
  `diagnosticScreenDecision` (`diagnostics/DiagnosticScreenData.kt:70-75`) are
  pure and tested; the screens only map enums to strings.
- The distinctions I13 asks for exist as data: `DiagnosticAttempt.blocked`
  (`diagnostics/DiagnosticRunData.kt:37-38`), `RunOutcome`, `EvidenceConclusion`,
  `MeasurementApplicability`, `probeUnavailable`. Nothing collapses a blocked
  condition into a failure any more (fixed in `4f6209e7`).
- The tiles are rendered from the measurement's own coverage layers
  (`ProtectionVerdict.kt:31-42`), so a retained measurement cannot be re-attributed
  to the current backend.
- The startup sequencing is deliberate and measured (reconcile after first
  paint, `startup/StartupCoordinator.kt:191-210`); the transitions below are a
  presentation problem, not a scheduling one.

### Where the model is wrong

**1. Eligibility is an admission decision that is being used as a display of
knowledge, and it erases what is known.** `diagnosticEligibility`
(`diagnostics/DiagnosticEligibilityData.kt:45,55`) returns `Checking` as soon as
a routing read is active or the observation is invalidated, *before* it looks at
`lastGood`. That is correct for the run coordinator (do not start probes on a
stale gate) but it means the presentation has no field that says "the last thing
we knew was VPN off". `diagnosticPresentation` reduces the whole routing
observation to `eligibility` plus one boolean `uncertain`
(`diagnostics/DiagnosticPresentation.kt:64-65`, `DiagnosticsCache.kt:115`). Every
downstream oddity in observations 2 and 4 follows from this: the hero cannot keep
"VPN is off" through a re-read because the projection threw the fact away.

**2. `Checking` is overloaded.** One enum value covers: inputs not supplied yet
(`current == null`, `DiagnosticPresentation.kt:65`), no observation ever taken
(`EligibilityData.kt:49`), invalidated and awaiting its debounced re-read
(`:55`), and a read in flight (`:45`); with any last-known value; for any reason
(VPN callback, config write, startup reconcile, foreground return, explicit
Retry, the run coordinator's own Checking/Verifying reads,
`diagnostics/AppDiagnosticRunIo.kt:36-40`). The same word is also a run stage
(`DiagnosticRunData.kt:7`). The hero's only way to distinguish these is
`measurement == null` (`DashboardHeroData.kt:123-125`), which is a property of
history, not of the read.

**3. `eligibility + cached tiles + note` is the wrong decomposition.**
`effectiveProtection` (`DashboardHeroData.kt:55-64`) overlays only the three
blocking eligibilities onto tiles that were cached at the last Dashboard
derivation (`DashboardData.kt:340`, `DashboardCache.kt:38`). `Checking` and
`Unknown` fall through to the cached `Checked` tiles, so `computeHeroStatus`
(`DashboardData.kt:384-420`) says Protected while the VPN state is precisely what
is unknown (observation 4). The note is a third channel bolted on to patch the
gaps of the first two: `ResultsUnverified` is declared non-downgrading
(`DashboardHeroData.kt:34`) so the routine refresh does not flicker, which is why
a Retry with the VPN off flashes green "VPN hidden / Confirming…" when an older
measurement exists (observation 2). The title comes from the cached verdict, the
subtitle from live knowledge, and the two are composed in the screen
(`DashboardScreen.kt:769`): "VPN hidden / Checking the VPN state…" is not a
bug in any one function, it is the product of the decomposition.

**4. The output type is a product of two enums that mostly do not combine.**
`HeroStatus` (4 values) x `HeroNote` (12 values) yields 48 combinations, of
which perhaps 12 are meaningful; the rest are prevented by convention
(`downgrades`, `explainsCondition`, the `measurement != null` guards) rather
than by type. `Attention` alone stands for: a warning banner, a partial layer,
restart pending, self excluded, a failed routing read, a checking state with no
history, an interrupted or failed attempt, a changed measurement, insufficient
evidence, a quarantined probe. Its default subtitle "Some checks need a look"
(`strings.xml:182`) is shown for restart-pending and self-excluded
(`DashboardHeroData.kt:128-132` returns `None`, so the status subtitle stays),
where no check needs a look at all; the self-excluded prompt underneath even says
"expected, not a failure" (`strings.xml:211`) under a yellow "Needs attention"
hero.

**5. The reason for a re-read is not modelled anywhere.** `ObservationRequest`
carries `id`, `generation`, `startedAt` (`ObservationData.kt:3-7`); an
`Invalidate(start=false)` from the VPN watcher (`VpnTransportWatcher.kt:150-153`)
leaves no timestamp and no cause. So the presentation cannot tell "the user
tapped Retry" from "the network changed" from "our own activation invalidated
the root snapshot" (`CanonicalConfigRepository.kt:42`), and cannot apply a
different wording policy to each. Observation 1 is exactly a Background re-read
worded as if the VPN state were in doubt.

**6. Two extra transitions come from the Dashboard being a root dependent.**
`DashboardCache` subscribes to `RootSnapshotCache.dependency`
(`DashboardCache.kt:23`), so every root invalidation (VPN toggle via
`refreshInPlace(force = true)` → `source.refresh()`, `StateCache.kt:69-72`;
reconcile; any save) re-derives the whole Dashboard and sets `loading`, which
drives the top-bar refresh indicator (`startup/MainActivity.kt:600`). That is the
spinner in observation 1. Separately, the second skeleton gate
(`DashboardScreen.kt:170-177`) replaces the entire screen with the loading
skeleton whenever the VPN comes up with no tiles yet, which is a much larger
visual transition than a hero state change.

**7. Retry does not retry.** `DiagnosticsCache.retry` reuses a completed suite
unless the measurement is `Changed` (`DiagnosticsCache.kt:140-142`,
`DiagnosticObservationData.kt:4-5`). A Retry / top-bar refresh with the VPN on
therefore re-reads the routing gate and re-derives the Dashboard but never
re-measures; the user sees "Confirming the last check still applies…" and the
same result. Whether that is intended is an open question (§4), but the hero
wording promises more than the action does.

**8. A recorded deviation from T12 is now user-visible.** A known VPN off → on
leaves the old measurement `MatchesLastObservation` (only config operations
advance the epoch, `MeasurementData.kt:72`; §19 boundary: `uncertaintyEpoch` not
implemented), so the hero returns to green "VPN hidden" on the strength of a
measurement taken under the previous VPN session. T12 asks for `Changed` with
history visible. This is not what the maintainer complained about, but any
redesign of the hero has to decide it (§4, open question 2).

Verdicts on the three questions asked: the decomposition is wrong (points 1, 3,
5); `Checking` is overloaded (2); the enum-pair output is the wrong type (4). None
of this is fixed by more notes.

## 2. Scenario table

"Today" is traced from the code on this branch. "Should" is under the
recommendation in §4 (Situation layer, read reasons, neutral Checking state,
Background grace of about 2 s). Colours: green = Protected, yellow = Attention,
red = Unprotected, grey = neutral (`surfaceVariant`, as VpnOff today).

| # | Scenario | Today (title / colour / subtitle / prompt, over time) | Should |
|---|---|---|---|
| 1 | Cold start, VPN on, self routed | Skeleton ~1.5–1.9 s → green "VPN hidden / All hiding layers are active" → ~0.4 s later green "VPN hidden / Confirming the last check still applies…" + top-bar spinner (reconcile invalidates root; routing re-read + Dashboard re-derive) → ~1 s later back to "All hiding layers are active" | Skeleton → green "VPN hidden / All hiding layers are active". The reconcile's re-read is Background: silent unless it exceeds ~2 s (then subtitle "Confirming…", colour unchanged) or changes the result. No top-bar spinner for a background re-derive. |
| 2 | Cold start, VPN off | Skeleton → grey "VPN is off / Hiding is inactive without a VPN" + VPN-off prompt → (reconcile) grey "VPN is off / Checking the VPN state…" → grey "VPN is off / Hiding is inactive…" | Skeleton → grey "VPN is off" + prompt; stays. Background re-read silent. |
| 3 | VPN off, then on, while on the Dashboard (complete measurement exists) | Off: green "Confirming the last check still applies…" + top-bar spinner ~1.75 s → grey "VPN is off" + prompt. On: grey → green "VPN hidden / Confirming…" (cached tiles win before the read; VPN-off knowledge lost) → green "All hiding layers are active" (old measurement presented as current, see §1.8) | Off: grey "Checking… / The network changed; checking the VPN state…" (Transition: immediate) → grey "VPN is off" + prompt. On: grey "Checking… / …checking the VPN state…" → then per open question 2: green (today's policy), or a confirmation suite ("Checking… / Running the hiding checks…") → green. |
| 4 | Retry with VPN off | No measurement: grey "VPN is off / Checking the VPN state…" + prompt stays → grey "VPN is off / Hiding is inactive…". With an older measurement: green "VPN hidden / Confirming the last check still applies…" → grey "VPN is off" | Grey "Checking… / Checking the VPN state…" immediately (Explicit), prompt's button shows progress → grey "VPN is off" + prompt. Never green. |
| 5 | Retry with VPN on, measurement exists | Green "VPN hidden / Confirming the last check still applies…" + top-bar spinner ~1.2 s → green; no re-measurement (`retry` reuses a completed suite) | Grey "Checking… / Re-running the hiding checks…" → green with fresh tiles (if open question 1 = re-run), else grey "Checking… / Checking the VPN state…" → green. Explicit action gets visible feedback and honest wording either way. |
| 6 | Background network handover (Wi-Fi ↔ cellular), VPN stays up | Green "Confirming the last check still applies…" + top-bar spinner ~1.75 s → green | Transition reason: grey "Checking… / The network changed; checking the VPN state…" ~1–1.75 s → green. Honest and rare while the Dashboard is visible; if judged noisy, raise the Transition grace (open question 3). No top-bar spinner. |
| 7 | Save of own roles during / after a run | Yellow "Needs attention / A configuration change is still being applied" → run Interrupted → yellow "…/ The VPN or configuration changed since the last check; run it again" (Changed) → user taps Retry → green "VPN hidden / Confirming…" during the re-run (cached tiles) → green or yellow | Grey "Checking… / Applying the configuration change…" (a wait, not a problem) → after settle: either an automatic confirmation run (open question 6) shown as grey "Checking… / Running the hiding checks…" → result; or yellow "Re-check needed / The configuration changed since the last check" + Re-check prompt. During any re-run of a Changed measurement: grey, never green. |
| 8 | Root revoked → routing read fails / suite fails | Routing failed: yellow "Needs attention / Couldn't determine whether VPN Hide is routed…", tiles stay, no prompt under the hero (prompt only on Diagnostics). Suite failed: yellow "…/ The latest check couldn't run; showing the previous result", no prompt (tiles are Checked) | Yellow "Couldn't check / Root access was denied or a command failed" + Retry prompt under the hero in both cases; tiles stay, marked as the previous result. Distinct from VPN off and from a leak (I13). |
| 9 | Interrupted run (context changed mid-run) | Yellow "Needs attention / The latest check was interrupted; showing the previous result"; if caused by a config change, immediately superseded by Changed | The cause wins: config change → the Changed situation (7); network change → the routing knowledge (3). "Interrupted" survives only as the Diagnostics notice beside the history. |
| 10 | Probe quarantine | Yellow "Needs attention / Diagnostic helper unresponsive"; Diagnostics banner says restart the app | Yellow "Couldn't check / The diagnostic helper did not return; restart the app". No Retry prompt (it would be rejected). Same. |
| 11 | Self excluded from the tunnel | Yellow "Needs attention / Some checks need a look" + self-not-routed prompt (whose text says this is expected, not a failure) | Grey "Can't check / VPN Hide isn't routed through the VPN" + the same prompt. No claim about hiding; not a warning. |
| 12 | Restart pending (self just added) | Yellow "Needs attention / Some checks need a look" + restart banner | Yellow "Restart to check / VPN Hide just enabled hiding for itself" + restart banner. Action-needed stays yellow; the subtitle names the action. |
| 13 | Cold start with VPN off, then VPN on (no tiles yet) | Whole screen replaced by the skeleton for the duration of the suite, then the full Dashboard | Layout stays; hero shows grey "Checking… / Running the hiding checks…", tiles show "—"; then green. |

Diagnostics screen, same scenarios: the banner is derived from the same
Situation, so 4 keeps the VPN-off prompt with a progress line instead of
swapping it for a bare spinner, 1 and 2 show no "Checking whether these results
still apply…" banner for a background re-read within grace, and 11 renders the
self-excluded prompt without a warning container.

## 3. Approaches

### A. Incremental: sticky last-known + read reasons + grace + fifth status

What changes: `ObservationRequest`/`ObservationState` gain a `reason` and an
`invalidatedAt`; `DiagnosticPresentation` gains `lastKnownRouting: SelfRouting?`
and `routingRead: (reason, since)?`; `effectiveProtection` maps
Checking-with-last-known-VpnOff to `Blocked(VPN_OFF)`; `HeroStatus` gains
`Checking`; `heroNote` learns to return `Checking` when the reason is Explicit or
Transition, or Background past grace; a delayed re-emission in the presentation
flow re-evaluates grace. `diagnosticScreenDecision` gets the mirror changes.

Deleted: nothing.

Scenarios: fixes 1, 2, 4, 6 and the green flash in 3. Leaves 5, 7 (green during
re-run of a Changed measurement, because `ResultsUnverified` is still
non-downgrading and the status still comes from cached tiles), 8 (no prompt), 11
and 12 (wrong subtitle) unless each is patched separately; every patch adds a
guard to `heroNote` or `effectiveProtection`.

Risks: the 4 x 13 enum product grows; hero and banner classifiers remain two
functions kept in agreement by hand; the tests stay one long assertion block
per function. Six months on, this is the same shape with more special cases.

Tests: extend `DashboardUiStateTest` and `DiagnosticScreenDataTest`; new
observation-reducer tests for reason/`invalidatedAt`.

Effort: about one day plus device pass.

### B. Typed hero and banner states from the presentation

What changes: A's plumbing (reasons, last-known routing, grace re-emission),
plus `HeroStatus`/`HeroNote`/`HeroDecision`/`effectiveProtection`/
`computeHeroStatus` are replaced by one pure `heroState(presentation, tiles,
issues, now): HeroState`, a sealed hierarchy with explicit knowledge cases
(`Hidden`, `Attention(reason)`, `Visible(reason)`, `VpnOff`, `NotMeasurable
(SelfExcluded)`, `ActionNeeded(RestartApp | ApplicationFailed | …)`,
`Checking(what, lastKnown)`, `CouldNotCheck(cause)`), each carrying its prompt.
`DiagnosticBanner` becomes a matching sealed `DiagnosticsBannerState` from a
second pure function.

Deleted: the two enums, `HeroDecision`, `effectiveProtection`, `heroNote`,
`showsFailedPrompt`, the second skeleton gate; `computeHeroStatus` folds into
`heroState` (issue counts become an input).

Scenarios: all thirteen, because the state is decided from knowledge first and
tiles second, and every case has its own wording slot.

Risks: two classifiers (hero, banner) still have to agree on precedence; the
scenario table has to be encoded twice; drift is possible but caught by
exhaustiveness.

Tests: one scenario-table test per function (thirteen fixtures, each asserting
the whole state), reducer tests as in A.

Effort: about two days plus device pass.

### C. One Situation, two wordings (recommended)

Same plumbing as A/B, but the classification is done once. A pure
`situation(presentation, now): Situation` folds knowledge and precedence into
one exhaustive sealed type; the hero and the Diagnostics banner are two thin
maps over it (`heroVisual(situation, tiles, issues)` and
`banner(situation, presentation)`), and the bundle/bridge can carry its name.

Sketch of the type (names are proposals):

```text
Situation
  Initializing                         // inputs not yet supplied
  Checking(what, lastKnown, reason)    // what: VpnState | Suite | ConfigApplying
  VpnOff                               // known, current
  NotMeasurable(SelfExcluded)          // known, current
  ActionNeeded(RestartApp | RestartDevice | ApplicationFailed | ApplicationUnknown)
  CouldNotCheck(RoutingUnknown | RunFailed(failure) | ProbeUnavailable)
  Measured(verdict, staleness)         // verdict: NoObservedLeak | OwnedLeak(partial|broken) | Insufficient
                                       // staleness: Current | Changed(cause) | Confirming(reason, since)
```

Precedence lives in `situation` only: process health (quarantine) → restart →
config readiness (Applying is `Checking`, Unknown/Failed are `ActionNeeded`) →
routing knowledge (Explicit/Transition re-read, or Background past grace, is
`Checking` with `lastKnown`; a failed read is `CouldNotCheck`; a known VpnOff /
Excluded is its own case) → run in flight (`Checking(Suite)` when there is no
applicable measurement, else `Measured(…, Confirming)`) → latest attempt failed
(`CouldNotCheck(RunFailed)` with history kept) → measurement verdict and
staleness. Errors/warnings from `dashboardIssues` are folded in by the hero map
(an error still outranks a `Measured` green), not by `situation`, because the
Diagnostics banner does not show them.

Hero wording map (one row per case; colour is a function of the case, never of a
note): `Measured(NoObservedLeak, Current)` → green; `Measured(_, Confirming)` →
green only for Background within grace (otherwise it is `Checking`), so the
"softening of §8" in §21 becomes a bounded, reasoned exception instead of a
non-downgrading note; `Measured(OwnedLeak)` → yellow/red as today via the
tiles; `Measured(_, Changed)` → yellow "Re-check needed"; `Checking` → grey
with a progress indicator in the icon bubble, title "Checking…", subtitle by
`what`; `VpnOff`, `NotMeasurable` → grey; `ActionNeeded`, `CouldNotCheck` →
yellow with the action in the subtitle and the matching prompt.

The Diagnostics map: `Checking` with a `lastKnown` condition renders that
condition's prompt with a progress line (no bare spinner over a known state);
`Measured(_, Confirming)` within grace renders `Ready`; `CouldNotCheck` renders
the retry prompt and keeps the history listed; everything else as today.

Grace: `situation` takes `now`; `Checking` vs `Confirming` for a Background read
is decided from `since + grace(reason) <= now`. The flow that publishes
`Situation` re-emits once at `since + grace` while the read is still active
(`transformLatest` + `delay`); it words, it never schedules a run, so I16 holds.
Proposed constants: Explicit 0 ms, Transition 0 ms, Background 2000 ms (the
reconcile's root reload with package inventory is ~1.2 s; 2 s covers a loaded
device without hiding a genuinely slow read for long).

Deleted: `HeroStatus`, `HeroNote`, `HeroDecision`, `effectiveProtection`,
`heroNote`, `conditionNote`, `showsFailedPrompt`, `computeHeroStatus` (folded),
`heroNoteRes`, the second skeleton gate in `DashboardScreen`, the `uncertain`
boolean, `DiagnosticBanner.Progress/ResultsUnverified/Failed/Interrupted/
RoutingUnknown/ProbeUnavailable` as separate enum values (they become cases of
`Situation`), `DiagnosticAttemptNotice` stays (it is a side channel beside
history). `DiagnosticEligibility` stays as the run coordinator's admission enum
and the bundle's vocabulary; it is derived, not rendered.

Scenarios: all thirteen by construction, and the hero and banner cannot
disagree on precedence because there is one precedence.

Risks: the largest diff of the three; `Situation` must not swallow per-surface
side channels (results list, attempt notice, tiles, issue counts stay as
separate inputs to the maps, or it becomes a god-type); the grace re-emission is
the only time-dependent piece and needs a test with a virtual clock; three
locales of new strings. Mitigation: stage it (§4) so each commit is small and
the old and new derivations can coexist for one commit each.

Tests: one scenario-table test for `situation` (the thirteen rows above as
fixtures, asserting the full case), one small test per wording map (exhaustive
`when`, so mostly prompt/colour assertions), reducer tests for reason and
`invalidatedAt`, a virtual-clock test for the grace re-emission, golden refresh
for the additive bundle fields.

Effort: about three days plus a device pass.

### Why C over B

B fixes the output type but keeps two independent precedence orders, which is
the mechanism by which the hero and the Diagnostics screen drifted apart on
this branch (the hero softened `ResultsUnverified`, the screen did not; the
hero has no Retry prompt for a failed routing read, the screen does). C spends
one more day to make that class of bug impossible and gives the bundle a
single name for "what the user was looking at", which `docs/debug-bundle.md`
triage would use directly. A is not recommended: it patches the three symptoms
observed today and leaves the model that produced them.

## 4. Recommendation and staged plan

Adopt C. Each stage is one reviewable commit that passes the full gate
(warnings-as-errors, ktlint, detekt, CPD, `lintDebug`, JVM tests; golden refresh
where noted). Order matters: stages 1–3 add without rendering, 4–6 switch the
surfaces, 7–8 clean up.

1. **Read reasons in the observation reducer.** `ReadReason { Explicit,
   Transition, Background }` on `ObservationEvent.Invalidate`/`Refresh` and on
   `ObservationRequest`; `ObservationState.stale: StaleMark(reason, since)?` set
   by invalidation, cleared when the current generation publishes or fails; a
   joined refresh upgrades the reason (Explicit > Transition > Background).
   Callers: `VpnTransportWatcher.trigger` → Transition; root-dependency
   invalidation and config-phase invalidation → Background; `ContextStateCache.
   refresh`, `RoutingGateCache.refreshIfStale` (foreground return; arguably
   Background — decide) → Explicit/Background; `AppDiagnosticRunIo.observe` →
   Explicit for a non-automatic run, Background for the automatic one. Tests in
   the observation reducer suite. No UI change.
2. **Routing knowledge in the presentation.** Replace `uncertain: Boolean` with
   `routing: RoutingKnowledge` (`Known(value, observedAt)`, `Verifying(lastKnown,
   reason, since)`, `Unknown(cause)`) built from `ObservationState<SelfRouting>`
   by a pure function; `eligibility` stays and is derived from it (same values,
   so the run coordinator and the bundle are untouched). Additive bundle fields
   on `DiagnosticSummaryInfo`: `lastKnownRouting`, `routingRead` (reason, since).
   Golden refresh, no schema bump (no field changes meaning). Tests:
   `DiagnosticPresentationDataTest` gains the knowledge cases.
3. **`Situation` and `situation()`**, with the scenario-table test, published as
   `DiagnosticsCache.situation: StateFlow<Situation>` including the single
   grace re-emission (virtual-clock test). Not rendered yet.
4. **Dashboard hero from `Situation`.** `heroVisual(situation, tiles, issues)`
   returns container/accent/icon/title/subtitle/prompt; `DashboardHeroCard` and
   the prompt block render it; the second skeleton gate goes; the `Checking` case
   shows a progress indicator in the icon bubble. Delete the enums and helpers
   listed in §3.C. `DashboardUiStateTest`: the hero block becomes a table over
   the thirteen fixtures. Strings (EN/RU/ZH): add `dashboard_hero_checking_title`,
   `dashboard_hero_checking_vpn`, `dashboard_hero_checking_suite`,
   `dashboard_hero_checking_config`, `dashboard_hero_recheck_title`,
   `dashboard_hero_recheck_subtitle`, `dashboard_hero_cannot_check_title`,
   `dashboard_hero_self_excluded_subtitle`, `dashboard_hero_restart_title`,
   `dashboard_hero_restart_subtitle`, `dashboard_hero_root_failed_subtitle`;
   remove `dashboard_hero_note_checking`, `dashboard_hero_note_results_unverified`,
   `dashboard_hero_note_interrupted`, `dashboard_hero_note_failed`,
   `dashboard_hero_note_results_changed`, `dashboard_hero_attention_subtitle` if
   no case still uses it (`lintDebug` has `UnusedResources = error`, so removals
   land in the same commit). Keep the RU copy idiomatic ("Проверяем…", not a
   calque of "Checking…"; see the copy rule in memory).
5. **Diagnostics banner from `Situation`.** `diagnosticScreenDecision` becomes
   `banner(situation, presentation)`; `DiagnosticBanner` shrinks to the cases the
   screen words differently from the hero; the VPN-off/self-excluded prompts get
   a `checking: Boolean` for the progress line. Strings: `diag_banner_results_
   unverified` stays for the slow Background case; add
   `diag_banner_checking_vpn`, `diag_banner_checking_suite`. Tests:
   `DiagnosticScreenDataTest` over the same fixtures.
6. **Top-bar refresh indicator from explicit reads only.** `DashboardCache.
   refreshing = observation.active?.reason == Explicit`; `MainActivity` uses it
   instead of `loading`. Removes the spinner in scenarios 1, 3, 6.
7. **Docs.** `docs/notes/app-state-transitions.md`: a §22 "Situation and read
   reasons" recording the bounded exception to §8 (Background grace) and
   superseding §21's "softening" paragraph; `docs/observation-coordinator.md`:
   replace the paragraph "For eligibility, an invalidated observation … is
   `Checking`" with the reason model and the hero/banner rule;
   `docs/debug-bundle.md` `diagnostics` row: the new fields and the `situation`
   name; `docs/diagnostics.md` §4: one sentence that the hero colour is decided
   by `Situation`, tiles by `LayerStatus`; `lsposed/AGENTS.md`: add `Situation`
   / `situation()` to the load-bearing list next to `DashboardIssue` with the
   same "never a `res.getString` in the classifier" rule;
   `docs/notes/state-refactor-handoff.md` item 5: closed by this work.
   Changelog fragment (user-visible): fixed — the Dashboard no longer says
   "VPN hidden" while the VPN state is being re-read, shows a neutral
   "Checking…" state instead, and no longer flips its subtitle at startup.
8. **Optional, after device pass:** the activator "runtime unchanged" marker so
   the startup reconcile skips its root invalidation entirely (handoff §Cold
   start). With stage 1–6 in place this is a performance item, not a UI one.

Device pass after stage 6: the thirteen rows above on the Pixel 8 Pro with a
screen recording, plus the Pixel 4a (Magisk, 4.14) for the self-excluded row.

### Open questions for the maintainer

1. **Explicit refresh with a completed measurement:** keep "reuse a completed
   suite" (today), or re-run on every explicit Retry / top-bar refresh? The suite
   costs ~0.15 s plus the reads; re-running is what the wording promises.
   Recommendation: re-run on explicit.
2. **Known VPN off → on with the same apparent identity (T12 vs today):** keep
   presenting the old measurement as current (today), mark it `Changed` until a
   re-check (T12, yellow after every toggle), or run one confirmation suite on
   the routed transition (an event, not a refresh, so I16 is not violated)?
   Recommendation: confirmation suite; it is the only option that is both
   honest and quiet.
3. **Transition grace:** 0 ms (recommended: the VPN may already be off when the
   callback fires) or align with the 750 ms debounce to hide handovers that
   resolve fast, at the price of claiming "hidden" for that long after a VPN-off
   toggle.
4. **Colour for `Checking`:** neutral grey with a progress indicator (recommended:
   "no claim", same family as VpnOff, distinguished by motion and icon) or an
   info-blue container as a fifth semantic colour. A fifth colour needs a
   `StatusColors` entry and a decision for every other "no claim" state.
5. **Self excluded as neutral instead of yellow** (row 11): the prompt text
   already says it is expected. Confirm.
6. **Own-roles save:** after a relevant operation settles, run a confirmation
   suite automatically, or keep `Changed` + manual re-check?
7. **Foreground-return re-probe (`refreshIfStale`):** Background (silent within
   grace) or Transition (visible)? Recommendation: Background; it is a safety
   net, not evidence of a change.
8. **Bundle:** the additive `lastKnownRouting` / `routingRead` / `situation`
   fields on `diagnostics` are fine without a schema bump? They do not change any
   existing field's meaning.
