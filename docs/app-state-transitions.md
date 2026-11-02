# App state: current transition contract

Core owns privileged operations and autonomous diagnostics. Kotlin owns the
process-lived read replica, intent/draft coordination and Android presentation.
The [historical contract](archive/2026-11-02-app-state-transitions.md) preserves
old transitions; its removed VPN gates and suite executor are not current APIs.
See [Core architecture](core-architecture.md), [config coordination](config-coordinator.md),
[observations](observation-coordinator.md) and [diagnostics](diagnostics.md).

## Owners

| State | Owner | Publication rule |
| --- | --- | --- |
| Canonical policy, journal, operation outcome and effects | Rust Core single writer | Persist intent before effects; committed policy and runtime application are separate. |
| Current/retained Core publication | Process-owned CoreSession reducer | Fence connection generations, epochs and revisions; disconnect withdraws current evidence. |
| Picker draft and Save/Cancel | Process-owned config coordinator | Keep local intent until confirmed commit/reconciliation; observations never write policy. |
| Root inventory and Android observations | ObservationCoordinator/adapters | Publish one immutable source packet; an older completion cannot replace a newer generation. |
| Diagnostic run, plan, fingerprint, assessment and cleanup | Core autonomous coordinator | Read-only consumers never admit a run; controlled workers submit observations. |
| Dashboard, agent and exported diagnostic read models | Accepted publication/source join | Retain historical evidence with its applicability; do not relabel capture time as measurement time. |

## Transition rules

| Event | Required behavior |
| --- | --- |
| Process/Activity creation | Attach to shared session and owners; Activity recreation does not create another writer, suite or recording. |
| Core unavailable or incompatible | Retain useful history as unavailable current evidence; expose bounded recovery/Retry. No empty-default write or false successful startup. |
| First compatible publication | Adopt its epoch/revision; join config/root/diagnostic identities only when compatible. No startup suite from opening the APK. |
| Picker toggle | Update the draft immediately; shared Rust/Core capacity and UID accounting prevent excessive selection. Keep the Save/Cancel footer. |
| Save | Allocate a Core operation ID and submit typed desired-value edits with the accepted base. Core validates again and owns all root phases. |
| Reply lost or timeout | Outcome is unknown; query the same operation ID. Do not blindly retry a write or roll back a newer user intent. |
| Conflict or known refusal | Reconcile the affected draft against accepted state and show the concrete reason; preserve unrelated edits. |
| Apply failed/unknown | Committed configuration remains distinct from Native/Ports delivery. Preserve holds and surface the affected resource. |
| Transport reconnect | Read the new publication under a new generation/epoch; stale replies cannot overwrite it. Uncertain mutations are queried, not replayed. |
| External VPN on/off or APK split-tunnel change | No Dashboard gate, main-process suite admission or VPN-required prompt. Connectivity observations remain only where real hook/fixture/forensic work needs them. |
| Diagnostic Retry | Coalesce pending UI intent, request Core admission, and consume the published result. Opening a screen or reading state does not request another run. |
| Export/log capture | Package retained evidence and bounded forensic capture under Core-owned leases. Do not start a fresh self-test or claim captured logs prove older measurements. |
| Worker/process/fixture failure | Keep confirmed leaks, incomplete controls, infrastructure and cleanup outcomes distinct. Missing RAW stimulus is not PASS. |

## Freshness and coverage

A retained observation is display history while loading, stale, failed or quarantined.
An accepted config revision is distinct from the general publication sequence;
unrelated read-model updates must not manufacture config conflicts. Core's
assessment and its source run are published atomically.

Native setup/runtime evidence, policy delivery and measured hook effectiveness are
separate. A fresh worker does not prove that old game processes consumed a new
Zygisk policy. A library mapping is not authorization or successful hook install.
Current diagnostics intentionally retain incomplete/unverified results; no missing
required fixture or process inventory may become a full-success verdict.

Fingerprint narrowing and separate lightweight policy health remain incomplete;
see [ROADMAP](../ROADMAP.md). Do not claim that all debug/capture-only publications
already avoid heavy cache invalidation. Core's required plan comes from supported
contracts and enabled features, not the installed mask or successful observers.

## Startup and lifecycle acceptance

Test cold Core readiness, competing root snapshot admission, reconnect and explicit
Retry on the final code. Normal ordering must not create
`core root snapshot probe exit=-1`; an actual failure must remain visible.
Provider discovery/boot-bound policy is under [renewed review](rfcs/0003-conditional-zygisk-publication.md).
Do not infer a new reboot/prompt/staging policy from these state rules.
