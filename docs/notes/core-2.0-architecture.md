# VPN Hide 2.0: persistent core architecture

Status: discussion record and proposed architecture, not an implemented contract.
Recorded 2026-09-17. Development is intended to proceed gradually on `dev-2.x`.
The next 1.x release must not depend on this migration. This document does not
authorize a VERSION bump, release, or replacement of the current implementation.

This captures the direction agreed during design discussion and the remaining
choices. Sections marked proposed require refinement before implementation;
especially IPC, authentication, observation sources, and upgrade recovery.

## 1. Direction and motivation

Adopt a persistent root core service, distributed as a common root-manager module.
It owns privileged configuration changes, backend selection and activation,
root observations, operation recovery, and root-side diagnostics. The app uses a
long-lived local connection instead of repeatedly staging helpers and invoking su.

The main benefit is one owner of decisions and invariants. Merely adding a daemon
that executes commands planned by the existing Kotlin coordinators would retain
most of the current complexity. Transfer ownership and remove superseded writers.

Expected benefits:

- Remove duplicated cross-module compatibility and activation coordination.
- Amortize root authorization, process startup, shell and repeated discovery work.
- Observe real VPN/routing changes independently of app-visible filtered APIs.
- Eliminate separate Ports and built-in bridge installation requirements.
- Make a separately loaded KPM manageable without its own companion activator,
  provided a compatible core is installed and the backend is accessible.
- Share Rust domain logic, typed clients and tests; progressively simplify Kotlin.

Costs remain: service startup/restart, authenticated IPC under Android SELinux,
independent version compatibility, durable operation recovery, resource bounds,
and a central management failure point. Performance gains must be measured.

## 2. Packaging and responsibilities

| Component | Intended responsibility |
|---|---|
| APK | Compose UI, Android APIs, LSPosed module, app-context diagnostics, core client |
| Common core module | Boot integration, service executable, root lifecycle and management |
| Built-in backend | Kernel implementation already present; no separate bridge module |
| kmod / KPM | Kernel implementation/artifact; core owns management and configuration |
| Zygisk module | In-process native hooks and root-manager loading integration |
| Ports | Core feature applying firewall rules; no separate installable module |

Removing activator binaries does not remove kernel hooks, Zygisk loading, or
LSPosed hooks. Backend packages can still need artifact installation and minimal
root-manager integration. Core must not depend on the APK process staying alive.
Native-only use must remain possible; LSPosed health is an independent capability.

Backend readiness comes from detected runtime identity, protocol compatibility and
capabilities, not merely the existence of a particular module directory. A raw
KPM is not automatically usable: loading, authorization, supported protocol and
conflict checks still apply. Package absence alone should no longer imply failure.

Keep local safety guards against conflicting kernel backends even when core owns
selection: manual root/module loading can bypass the manager.

## 3. Owners and state model

Preserve three distinct concepts:

1. **Desired configuration:** durable canonical user intent, owned by core.
2. **Application evidence:** operation phases and which backend received/applied
   which configuration, owned by core; significant operation evidence is durable.
3. **Observed facts:** root-side measurements owned by core and app-context
   measurements produced in the app. Observations have freshness and provenance.

Saved does not mean applied; applied does not mean every process consumed it;
neither implies that a diagnostic check passed. Preserve known success, known
failure, unattempted work and unknown outcome as distinct states.

Proposed initial storage: retain `/data/system/vpnhide_config.json` to preserve
existing consumers, with core as the sole normal writer. Operation metadata is
separate. Inventory every remaining boot script, CLI, agent-control action and
manual activation path before claiming exclusive ownership. LSPosed can retain
direct read access to a compatible published config; never block hook callbacks
on a synchronous RPC to core.

Core keeps working state in memory and acknowledges persistence only after the
required durable write. On restart it loads configuration and operation records,
then re-observes runtime state. Old observations are not automatically current.
Malformed/unavailable canonical data must never silently become default config.

The app owns UI/navigation, local preferences, editor drafts, pending intent,
local Android API execution, and a read-only replica of confirmed core state.
Kotlin and an optional Rust client must not become competing canonical writers.

## 4. Configuration commands and operation lifetime

Send typed field intents, not an authoritative replacement of a screen snapshot.
Commands include a stable operation ID and conflict-checking base information.
Core prepares each accepted edit against fresh state. Unrelated field changes
can merge; stale changes to the same field produce an explicit conflict. Whole
import/reset has a broader write set. Preserve existing self-target policies.

Conceptual API, not a frozen wire schema:

```text
submit_edit(operation_id, base, edit) -> accepted / rejected
get_operation(operation_id)          -> phases and outcome
watch_state(cursor)                  -> current snapshot / changes
```

Persist confirmed config before publishing it as confirmed. Activation failure
after persistence keeps that config and offers explicit repair. An unchanged
config may still need activation repair without another canonical write.

Cancellation of an RPC waiter, Activity destruction, socket loss or client death
does not cancel a durably accepted mutation. Core owns it independently. Repeating
an operation ID with the same payload queries/deduplicates the operation; reuse
with a different payload is rejected. Specify bounded record retention and an
explicit expired/unknown-ID policy that cannot accidentally replay an old request.
Do not promise exactly-once external effects across crashes without evidence.

Preserve the current helper's guarantees: durable evidence before dispatch,
single mutation ownership, late-dispatch fencing, descendant lifetime accounting,
bounded output, and no blind replay after an unknown outcome. A daemon restart
alone does not prove old workers or descendants stopped. A singleton lock alone
is insufficient. Prefer supervised workers for potentially blocking/external
effects; subreaper responsibilities stay out of JNI and system_server.

Recovery may reconcile observed state; it must not invent completion or re-run
uncertain destructive operations. Reset must preserve the metadata needed to
fence predecessors. Define upgrade, shutdown and worker ownership before replacing
the current one-shot supervisor.

## 5. State publication and synchronization

Use commands in one direction and authoritative snapshots/events in the other;
this is not bidirectional database synchronization.

- Initial subscription returns a snapshot and subsequent updates without a gap.
- Track config revision, relevant observation generations, operation IDs, and a
  publication cursor scoped to a core instance/epoch separately.
- A stats update must not conflict with an editor or invalidate unrelated tests.
- On disconnect retain last-good display data but mark current evidence unavailable.
- Reconnect can start with a fresh snapshot. Durable event replay is not required
  for the first implementation; operation outcomes remain separately queryable.
- Coalesce replaceable state updates for slow consumers; bound queues and reject
  stale publications. Never rely on an event stream as the sole operation receipt.
- An invalidation can be published before the replacement observation is ready.
  Preserve Checking versus failed/Unknown and last-good versus current semantics.

Draft text remains local. Proposed preservation of today's draft protection:
register field-domain reservations with bounded leases, ownership and explicit
release. Expiry/disconnect cannot hold the core forever. Saving after reservation
loss still performs base/conflict checks. Specify multi-client behavior and test
UI versus CLI/agent-control conflicts; leases are not yet a final API decision.

## 6. Mapping existing app coordinators

| Existing responsibility | Intended destination |
|---|---|
| Config preparation, persistence, phase ordering, recovery | Core Rust domain/coordinator |
| Editor drafts, optimistic switches, acknowledgement, feedback | App UI layer |
| Root snapshot collection, invalidation, coalescing and quarantine | Core observation layer |
| Local projections and stale-computation rejection | App/client observation layer |
| Java checks and native checks in the actual app process | APK Kotlin/JNI |
| Root differential checks and privileged collection | Core/workers |
| Diagnostic session context and shared classification rules | Shared typed model; precise coordinator boundary still to define |

Reuse existing reducers and behavioral tests as specifications. Do not port
everything to Rust merely because a Rust client exists. Android lifecycle,
permissions, PackageManager UI data, Compose, and Java callback measurements still
have natural Kotlin owners. Preserve field-specific merge, cancelled-waiter,
unknown-outcome, generation and quarantine guarantees through each migration.

## 7. Diagnostics and real VPN observation

Root cannot replace app-context measurements. An app_process probe under the same
UID is not equivalent to the real target process's loaded hooks and SELinux
context. Keep JNI probes in the APK process and Java checks through Android APIs.
Reuse common native check logic, but execute it in both relevant contexts.

Associate measurements with diagnostic session, config/apply context and relevant
network generations. Revalidate at completion. A change during the run can make
the measurement inapplicable; do not mix unrelated old app and new root results.
Confirmed leaks must not be hidden by incompleteness in a different check.

Core should observe real network/routing changes without consuming its own
sanitized app callbacks. Exact observer implementation is open: kernel link/route
events can trigger remeasurement but a tunnel interface alone is not proof that
this UID is routed through a VPN. Account for split routing, missed events, initial
resync and event-source failure. An LSPosed signal can be an additional trigger;
it must not become the sole truth for native-only installations.

Temporary self-filter toggling was considered and is not preferred: it introduces
config/event races and changes the diagnostic environment. Exempting the entire
app from callback filtering weakens self-tests. A private watcher registration
exception would require a trustworthy identity and is not selected.

## 8. IPC choices: deliberately unresolved

Agreed direction: local UDS connection between APK and root service. No final RPC
library, codec, socket path, authentication or JNI binding technology is selected.
The existing kernel control/telemetry protocol is a different boundary and is not
replaced merely because core IPC changes. Keep its codegen and protocol-diff tests.

Client location is also open:

- **Kotlin:** direct coroutine/Flow/lifecycle integration, fewer JNI calls; needs a
  suitable RPC/UDS adapter and generated or shared-schema Kotlin types.
- **Rust inside the APK .so:** shared SDK for APK, CLI and host tests, shared API
  types and reconnect logic; requires an async JNI boundary and Kotlin-facing
  types. Existing native probes alone do not justify this choice. Never create
  a second authoritative config coordinator in the SDK.

Rust client was favored if it owns a meaningful reusable SDK. A single APK .so
can contain local probes and the client. It still runs without root; using Rust
does not bypass socket permissions or SELinux.

| Candidate | Benefits | Costs/questions |
|---|---|---|
| tarpc over UDS | Rust-defined service and types, generated client/server, deadlines/cancellation, no .proto pipeline | Choose serialization and compatibility rules; design observation delivery, e.g. long-poll by revision |
| tonic over UDS | gRPC/HTTP2, generated API, native streaming RPC, protobuf evolution rules | HTTP2 stack/dependencies, .proto build step, mapping to rich Rust domain types; measure footprint |
| jasonrpc over UDS | JSON-RPC 2.0, readable messages, documented multiplexed UDS client and framing | Typed calls are not a generated shared service contract; audit notifications/subscriptions, bounds and disconnects |
| Custom protobuf framing over UDS | Explicit cross-language schema | Must supply RPC infrastructure; not preferred over evaluating tonic first |

Rust-defined public types are a valid single source of truth. Protobuf is not
required just because APK and core update independently. Separate public wire
types from private implementation, define supported version skew and capabilities,
and test old/new combinations whichever library is chosen. Core module version,
APK version and backend wire version must not be conflated.

Tarpc is attractive for Rust-only request/reply plus `watch_state(after_revision)`
long polling. Tonic is attractive when multiple event/progress/log streams are
first-class. This discussion did not select either. Avoid choosing on hypothetical
performance: measure release APK/service size, startup, RSS and idle wakeups.

Prototype acceptance: authenticated app-originated connection under enforcing
SELinux; request/reply and subscription; disconnect/restart and snapshot recovery;
slow client and bounded buffers; mixed versions; operation surviving client loss.
Knowledge of a socket name is not authorization. Define peer identity, package
reinstall/UID changes, user/profile scope and root-manager authorization/revocation.
An APK update must not let an untrusted binary become root service code. Minimize
policy changes; no world-writable control endpoint or arbitrary shell RPC.

References checked during discussion:

- [tarpc](https://github.com/google/tarpc)
- [tarpc UDS](https://docs.rs/tarpc/latest/tarpc/serde_transport/unix/index.html)
- [tonic](https://docs.rs/tonic/latest/tonic/)
- [tonic UDS example](https://github.com/grpc/grpc-rust/tree/master/examples/src/uds)
- [jasonrpc](https://docs.rs/jasonrpc/latest/jasonrpc/)
- [JSON-RPC 2.0](https://www.jsonrpc.org/specification)
- [protobuf evolution](https://protobuf.dev/programming-guides/proto3/#updating)

## 9. Traffic and resource expectations

One initial snapshot, commands on committed user actions, and updates when
relevant facts change. Editor keystrokes stay local. Statistics subscriptions have
a bounded cadence and only run when needed; diagnostics and export are explicit
heavier workloads. Suspend app subscriptions when not useful; core still manages
the system without an app connection. An idle connection need not poll constantly.

No RPC per packet, syscall, hook invocation or app network callback. Hooks use
locally applied policy. Avoid whole-inventory refresh on each network transition.
Use bounded collection workers, deadlines, coalescing and resync after lost events.
Large bundles must not block small control messages or imply unbounded buffers.

## 10. Ports and module migration

Ports implementation already resides in `crates/activator/src/ports.rs` and
`lifecycle.rs`: UID policy projection, IPv4/IPv6 restore, OUTPUT jumps, boot apply
and uninstall cleanup. Move it behind a core backend adapter; remove the separate
ZIP only after migration ownership is explicit.

Current boot code waits for `bw_OUTPUT`, applies once, then repeats after 30s.
A persistent core can reconcile missing rules, but needs verified triggers or a
bounded inspection policy; persistence alone does not detect netd rule rebuilds.
IPv4/IPv6 changes are not one atomic transaction. Report partial outcomes honestly.
Rules can survive core failure; restart must inspect rather than assume absence.

Old Ports uninstall removes chains: do not let it erase rules already owned by
core. Migration must stop legacy boot writers, order cleanup/adoption and survive
interruption. The built-in bridge similarly disappears only after core owns its
boot tasks. Existing KPM/kernel/Zygisk consumers need compatible policy delivery.

Plan common-module disable/uninstall, absent core, interrupted upgrades, rollback,
root-manager differences, APatch secrets, logging leases and full reset explicitly.
Core failure must be visible and must not be presented as disabled protection:
already-applied backend behavior can persist. Preserve a restricted maintenance
path for recovery without reintroducing independent ordinary writers.

## 11. Proposed incremental development sequence

1. Finalize ownership and IPC/authentication decisions, then device prototype.
2. Add shared domain/API and core packaging/lifecycle, typed read-only client.
3. Implement root observations and consistent state subscription; integrate UI.
4. Transfer durable config operations and recovery with tests proving old guarantees.
5. Move Ports and activation adapters under core; migrate installed module state.
6. Integrate app-context diagnostics and capture with core session provenance.
7. Remove obsolete activators, bridge/Ports packages, writers, caches and UI checks.
8. Validate upgrade/restart/rollback and independent version combinations before 2.0.

Each boundary needs an explicit cutover. Do not run legacy and core writers in
parallel against the same canonical/runtime state. Port behavioral tests before
deleting them. Include killed service/worker, delayed dispatch, multiple clients,
same-field conflict, reset, slow subscriber and backend partial-failure scenarios.
Test built-in first on the available device, then supported kmod/KPM/Zygisk/root
manager combinations; one device cannot establish the entire support matrix.

## 12. Independent near-term 1.x fix

The immediate bug: Dashboard can miss VPN-up because a correctly hidden network
transition does not necessarily produce a callback for the app's own UID. Retry
can read the real routing state. Do not delay the next release for core.

Options discussed: manual Retry only; foreground-return refresh; foreground-only
timer; private LSPosed trigger; trusted per-registration watcher exemption.
Proposed pragmatic release path is reuse the existing resume check plus bounded
foreground polling, independently fixing callback symmetry. Exact implementation
and interval remain unapproved/unimplemented; 3 seconds was an initial suggestion.

Source inspection on 2026-09-17 (main `b8f4df41`, plus relevant files in
`refactor/ui-situation` at `54bd755e`):

- `startup/MainActivity.kt` already calls `RoutingGateCache.refreshIfStale()` on
  `ON_RESUME`.
- `diagnostics/RoutingGateCache.kt` uses a 4000 ms throttle since the start of the
  last load; it is not a periodic timer and is a no-op before initialization.
- `diagnostics/VpnTransportWatcher.kt` registers VPN/default callbacks and debounces
  refresh triggers by 750 ms. No periodic VPN poll was found in these current paths.
- Other timers (statistics, capture elapsed time, hook-service attachment) are not
  a VPN-state polling fallback. Trace fresh root inputs before reusing a refresh
  method: repeatedly reading a cached root snapshot is not fresh VPN observation.
- Watcher comments and observation documentation assume default delivery on both
  VPN edges; the reported suppressed up-edge invalidates that assumption. Source
  inspection here is not a new device reproduction or a completed fix.

Polling must be foreground-bounded, single-flight and independent of suppressed
callbacks; use the minimal fresh routing measurement, not a full suite/inventory
on every tick. Unknown is not VPN-off. Preserve retry/quarantine/resource bounds.

Callback hiding has a separate invariant: if the app-visible network and properties
do not change, neither VPN-up nor VPN-down should produce a spurious lifecycle
transition. Preserve actual Wi-Fi/cellular, offline/recovery and initial delivery.
Track each registration independently; inspect capabilities/link-properties
duplicates as well as Available/Lost. No claim of eliminating every timing side
channel follows from this invariant.

Device acceptance must include VPN on/off over unchanged Wi-Fi, Wi-Fi/cellular,
offline/recovery, multiple registrations, background/foreground and manual Retry.
Install the matching APK and reboot for system_server hooks before testing.

## 13. Existing contracts to preserve and update at implementation time

- [Config coordinator](../config-coordinator.md)
- [Observation coordinator](../observation-coordinator.md)
- [Mutation transport and descendant guarantees](../root-mutation-transport.md)
- [State transitions](app-state-transitions.md)
- [Diagnostic semantics](diagnostics-state-analysis.md)
- [Storage and activation](../storage.md)
- [Persistent paths and ownership](../state.md)
- [Backend protocol](../protocol.md)
- [Capacity/resource limits](../limits.md)
- [Diagnostics](../diagnostics.md)
- [Debug bundle](../debug-bundle.md)

These describe the current system, not completed 2.0 migration. Update them when
ownership actually changes; this discussion record alone does not supersede them.
