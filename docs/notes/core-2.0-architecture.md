# VPN Hide 2.0: persistent core architecture

> **2.0 status (implemented on `dev-2.x`).** This was the design record; the
> implementation now exists and differs from it in places. Where they disagree, the
> code and the current docs ([storage.md](../storage.md), [state.md](../state.md),
> [core-2.0-module-lifecycle.md](core-2.0-module-lifecycle.md)) win. Settled since:
> core is a required module and the only root process; the app has no `su` and runs
> no shell; there is no shell logic on the device (`service.sh` is one `exec`);
> native selection is strict (more than one candidate loads nothing); the 1.x
> activators, Ports module and Built-in companion module are removed. The 1.x
> mutation transport linked below exists only on `main`.

Status: discussion record and proposed architecture, not an implemented contract.
Recorded 2026-09-17; revised the same day after a design session that read the
current in-process coordinators and resolved several choices the first draft left
open. Development is intended to proceed gradually on `dev-2.x`. The next 1.x
release must not depend on this migration. This document does not authorize a
VERSION bump, release, or replacement of the current implementation.

Where the first draft treated a choice as open, this revision either resolves it
against the existing code or marks it as still needing a device spike. Two things
still need a prototype before implementation: the observation value that justifies
the daemon at all (§8), and the SELinux/socket plumbing across the three root
managers (§9). Everything else here follows from the state-ownership decision in §3.

## 1. Direction and motivation

Adopt a persistent root core service, distributed as a common root-manager module.
It owns privileged configuration changes, backend selection and activation, root
observations, operation recovery, and root-side diagnostics. The app uses a
long-lived local connection instead of repeatedly staging helpers and invoking su.

The single honest justification for a persistent daemon is **observation the
hidden app cannot perform for itself.** The app's own network callbacks are
filtered and can be suppressed (that is the 1.x bug in §13), the app is not running
at boot, and it is killed constantly. Only a persistent root process can watch real
routing, finish an interrupted operation, and manage a backend while the app is
dead. The other benefits — amortized su, deduplicated compatibility logic — are
optimizations, not new capability, and do not by themselves justify turning an
in-process monolith into a client/server system.

So the go/no-go for 2.0 is one question: is independent, event-driven observation
of real routing (a) reliably achievable and (b) worth the distributed-systems cost?
That must be answered by a spike (§8, §12), not by architecture.

The cost is smaller than a naive reading suggests, because the coordination it
would seem to require **already exists and is correct** in Kotlin. `ConfigCoordinator`
and `ObservationCoordinator` (§3) already implement generation supersession,
last-good-versus-current, quarantine, durable phase ordering, recovery, and
"cancelling a caller never cancels an accepted operation." The migration relocates
the *owner and durable anchor* of that logic; it does not invent it. The genuinely
new work is a connection-lifecycle layer and a cross-process epoch (§6).

Merely adding a daemon that executes commands still planned by the Kotlin
coordinators would retain today's complexity and add a socket on top — strictly
worse. The point is to transfer ownership of the state that must outlive the app,
and remove the superseded writers.

## 2. Packaging and responsibilities

| Component | Intended responsibility |
|---|---|
| APK | Compose UI, Android APIs, LSPosed module, app-context diagnostics, core client, read-replica of core state |
| Common core module | Boot integration, service executable, root lifecycle, canonical ownership, observation loop |
| Built-in backend | Kernel implementation already present; no separate bridge module |
| kmod / KPM | Kernel implementation/artifact; core owns management and configuration |
| Zygisk module | In-process native hooks and root-manager loading integration |
| Ports | Core feature applying firewall rules; no separate installable module |

The native backends are **mutually exclusive**: the user runs exactly one of
kmod / KPM / built-in / Zygisk. This is why observation cannot be delegated to "the
kernel backend" — on a Zygisk install there is no kernel vantage at all (see §8).

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

## 3. State ownership: what belongs to core, what stays in the app

The first draft asked whether the app or core "owns state." That framing is wrong;
the decision follows from one test.

**The test: what must be true when the app process is dead?** Boot activation, the
recovery of an operation the user abandoned mid-apply, and the netlink observation
loop all run with no JVM alive. Therefore the facts they need — canonical config,
in-flight operation state, observations — must be owned by core. This is a
consequence, not a preference. It also rules out a "dumb executor" core: a core
that only runs Kotlin-planned commands cannot boot-activate, cannot recover after
app death, and cannot hold a netlink socket, so those jobs would fall back to
separate boot scripts and one-shot recovery — the split-brain §11 forbids.

Ownership therefore splits by **who can physically produce a fact**, into three
buckets:

**A — core is source of truth** (durable, must survive app death, measurable only
under root): desired configuration (canonical intent); operation evidence, phases
and recovery; root observations (real routing, VPN presence, backend liveness,
root-differential). The app holds a **read-only replica**; edits are submitted, not
written locally.

**B — the app is source of truth** (app-local, or measurable only in the app's own
process): editor drafts, UI/navigation, local preferences; and the app-context
measurements — JNI probes and Java/framework checks that require the app's real
uid, SELinux domain and loaded hooks, which core physically cannot reproduce. Core
never sees a draft until submit, and stores an app check result only as
provenance-tagged foreign evidence, if at all.

**C — joined verdict** (needs both halves at once): the diagnostic verdict is a join
of core's root observation and the app's in-process check. It is built in one place
(the app's existing report builder, which already renders rather than recomputes),
with provenance on each half, and never recomputed by consumers. Core's own
decisions must depend only on bucket A, never on a bucket-C verdict — that is what
lets core act while the app is dead.

**The app already is this core, in Kotlin, in-process.** The migration moves the
fence; it does not redraw it:

- `ConfigCoordinator` is an actor that already owns canonical writes, phase ordering
  (`Persist` / `Native` / `Ports` / `Cleanup` / `Secret`), recovery/reconcile,
  drafts and operation IDs, and already guarantees that cancelling a caller never
  cancels an accepted root operation. Its `view` (`ConfigCoordinatorView`) is already
  a read-replica that publishes `confirmed` only when the config is available.
- `ObservationCoordinator` is a pure reducer (`reduceObservation`) plus effects and
  an actor, already carrying generations, `lastGood` versus current
  (`currentObservationValue`), and quarantine/drain/recover.
- Both talk to the world through narrow ports — `ConfigCoordinatorIo`
  (`initialize` / `read` / `execute` / `recover`) and `ObservationDependency`
  (`subscribe` / `refresh`) plus a `load(request)` — and `StateCache` /
  `ProjectedStateFlow` already express "projections read one immutable source, no
  second publishing job."

So 2.0 relocates the durable anchor of bucket A into core and swaps the port
implementation from su-shell orchestration to socket RPC; the app keeps `view` as
its replica, keeps drafts and app-context checks local. "Simplify Kotlin" means
delete the su orchestration and section-grepping, not rewrite the coordination.

## 4. Configuration storage and activation ownership

Three distinct kinds of state, often conflated into "the config file", pull apart:

**1. The config file is the durable canonical config.** `/data/system/vpnhide_config.json`
is where the canonical config lives on disk: core is its **sole writer** and **reads it
back on restart** (recovering the config after a reboot). It is at the same time the
source **LSPosed reads directly** — an RPC-less reader, because its hooks run in
system_server and must never block on a call to core (§3). So the file is both core's
config store and LSPosed's read source: **input and output for core**, not a write-only
projection. It is written atomically (write-temp + rename, so readers get whole-old or
whole-new) and its schema is the frozen shape LSPosed already parses. DAC makes it
**output-only for everyone but core** (`root:root`, `0640`, others read) — which is what
makes the single-writer/dual-writer guarantee in §11 enforceable rather than conventional.

There is no *separate* private store for the config: the config the app edits is exactly
what the readers consume, so a second copy would be pointless. "Adopt the legacy file on
first start" is therefore just the ordinary restart read, not a special one-shot.

**2. Backend activation is core's job, done in-process — core replaces every activator.**
Core does not publish a file "for the activator to read"; there is no separate activator.
Holding the canonical, core derives the per-backend kernel wire (docs/protocol.md v2, a
frozen boundary) and the Ports UID policy and **applies them directly** to the one active
backend — writing the kernel control channel, setting iptables — on a config change and at
boot. These are **apply-actions, not published files**: nothing here is ever read back
(the "never read a projection as truth" rule applies here, trivially, and *not* to the
config file, which is the canonical). The activator's projection logic (`crates/activator`
— `project_native`, `project_ports`, `PackageUidMap`) moves **into** core; the separate
activator binaries and su activation paths retire. This is "backend selection and
activation, owned by core" (§1), and it lines up with core driving the single active
backend (the natives are mutually exclusive).

**3. Operation / recovery / epoch metadata is core's private durable store.** Operation
phases, recovery evidence and the publication epoch are **not** config and do not belong in
the LSPosed-facing file (§3: operation metadata is separate). This is the state that lives
in core's own format, is free to evolve, and is also read back on restart. The "free to
evolve internal format" benefit is here — not in the config file, whose schema is pinned by
its readers.

Consequences, several of which the current code makes mandatory:

- Persist the config durably **before** publishing it as confirmed.
- `rename()` solves torn reads; reader **staleness** (when LSPosed re-reads a changed file)
  is a separate question and must be answered (inotify, or a cheap re-read on the relevant
  callback — never a synchronous RPC to core from a hook, §6).
- Current cleanup this forces: the app stops grepping the config out of the root su-snapshot
  (`canonical_config` in `RootSnapshotCache`) and reads it as a typed field of core's
  published state instead (`StateSnapshot.config`).

Inventory every remaining boot script, CLI, agent-control action and manual activation path
before claiming exclusive ownership. Malformed or unavailable config must never silently
become a default config.

## 5. Configuration commands and operation lifetime

Send typed field intents, not an authoritative replacement of a screen snapshot.
Commands carry a stable operation ID and conflict-checking base information. Core
prepares each accepted edit against fresh state (as `ConfigCoordinator` already does
in `prepare` via `io.read()`, merging the real write set only once it is known).
Unrelated field changes can merge; stale changes to the same field produce an
explicit conflict. Whole import/reset has a broader write set. Preserve existing
self-target policies.

Conceptual API, not a frozen wire schema:

```text
submit_edit(operation_id, base, edit) -> accepted / rejected
get_operation(operation_id)          -> phases and outcome
watch_state(cursor)                  -> current snapshot / changes
```

The guarantees below are **already implemented** in `ConfigCoordinator`; the task is
to preserve them across the process boundary, not to design them:

- Persist confirmed config before publishing it as confirmed; activation failure
  after persistence keeps that config and offers explicit repair; an unchanged config
  may still need activation repair without another canonical write.
- Cancellation of an RPC waiter, Activity destruction, socket loss or client death
  does not cancel a durably accepted mutation. Core owns it independently.
- Durable evidence before dispatch, single mutation ownership, late-dispatch fencing,
  descendant lifetime accounting, bounded output, and no blind replay after an
  unknown outcome. A daemon restart alone does not prove old workers or descendants
  stopped; a singleton lock alone is insufficient. Prefer supervised workers for
  potentially blocking/external effects; subreaper responsibilities stay out of JNI
  and system_server.

New at the process boundary: repeating an operation ID with the same payload
queries/deduplicates it; reuse with a different payload is rejected. Specify bounded
record retention and an explicit expired/unknown-ID policy that cannot accidentally
replay an old request. Do not promise exactly-once external effects across crashes
without evidence. Recovery may reconcile observed state; it must not invent
completion or re-run uncertain destructive operations. Reset must preserve the
metadata needed to fence predecessors.

## 6. State publication and synchronization

Use commands in one direction and authoritative snapshots/events in the other; this
is not bidirectional database synchronization. Most of the machinery below already
exists inside `ObservationCoordinator`/`ObservationState`; the migration adds only a
connection-lifecycle layer and a cross-process epoch on top of a correct reducer.

- Initial subscription returns a snapshot and subsequent updates without a gap.
- Track config revision, relevant observation generations, operation IDs, and a
  publication cursor scoped to a **core instance/epoch**. This epoch is the one new
  concept: today `generation` is per-cache and in-process; a restarted daemon is a
  new epoch, and the replica must notice rather than splice an old generation onto a
  new instance.
- A stats update must not conflict with an editor or invalidate unrelated tests.
- On disconnect retain last-good display data but mark current evidence unavailable
  (`ObservationState` already distinguishes last-good from current). **Daemon
  unreachable is Unknown — not VPN-off and not "protection disabled."**
- Reconnect can start with a fresh snapshot. Durable event replay is not required for
  the first implementation; operation outcomes remain separately queryable.
- Coalesce replaceable state updates for slow consumers; bound queues and reject
  stale publications. Never rely on an event stream as the sole operation receipt.
- An invalidation can be published before the replacement observation is ready.
  Preserve Checking versus failed/Unknown and last-good versus current semantics.

Draft text remains local (bucket B). If cross-client draft protection is needed at
all, register field-domain reservations with bounded leases, ownership and explicit
release; expiry/disconnect cannot hold core forever, and saving after reservation
loss still performs base/conflict checks. Leases are not a final API decision.

## 7. Where the coordination logic runs: Rust versus Kotlin

Two independent questions were being conflated: *who owns the state* (§3, answered:
core) and *what language the reducer runs in*. The language follows from ownership —
**the reducer lives where its state lives** — which forces most of it:

- The config-operation reducer governs core-owned lifecycle that must run with the
  app dead (boot, recovery, a netlink event with no JVM). It therefore runs in the
  daemon → **Rust. Not optional.** Keeping it in Kotlin would put the authoritative
  logic in a process the authoritative daemon cannot call.
- The app-local reducers — package inventory, app-context checks, UI projections —
  are bucket B, run only while the app is alive, and are JNI-free JVM code that
  shares nothing with core or the CLI. They stay in **Kotlin. Also not optional**;
  routing them through an async Rust boundary adds friction for no gain.
- The generic `ObservationCoordinator` state machine is instantiated by **both**
  sides (root/routing in core, package/app-context in the app), so the pattern
  unavoidably exists in both languages. This is the only genuinely open sub-choice:
  one shared Rust implementation (called by daemon, CLI, host tests, and the app via
  its `.so`) versus two idiomatic implementations.

Recommendation: keep **two** implementations of the generic machine and control drift
with **one language-neutral behavioral test vector** — a table of
`(events → expected states)` both the Rust and Kotlin reducers must pass. This is
the "reuse reducers and behavioral tests as specifications" principle made concrete: the spec
is the test vector, not shared code, and drift is caught by CI rather than
discipline. Duplicating a ~200-line pure reducer is cheap; forcing the app's
JNI-free caches through Rust is not. The app-side choice is decided last in the
sequence (§12) and may simply remain the existing Kotlin.

Rust domain logic already exists and is substantial (the activator, app-helper and
checks crates), so a Rust core reuses an SDK rather than starting one.

## 8. Diagnostics and real VPN observation

Root cannot replace app-context measurements. An app_process probe under the same
UID is not equivalent to the real target process's loaded hooks and SELinux context.
Keep JNI probes in the APK process and Java checks through Android APIs (bucket B).
Reuse common native check logic, but execute it in both relevant contexts. Associate
measurements with diagnostic session, config/apply context and relevant network
generations; revalidate at completion; a confirmed leak must not be hidden by
incompleteness in a different check.

**The observer is the daemon, and it is backend-agnostic by construction.** Because
the daemon runs as uid 0 and the backends filter per target uid, a root observer sees
the unfiltered kernel truth under *any* chosen backend — and this is the reason it
cannot be delegated to the backend: the backends are mutually exclusive, and a
Zygisk-only install (in-process libc hooks in the target) offers no routing vantage
at all. The worry about "consuming your own sanitized callbacks" does not
apply to a uid-0 netlink listener; it is not filtered.

What the daemon adds over today: root observation is currently one large su shell
script (`resources/shell/root_snapshot.sh`, ~40 sections including `canonical_config`),
pulled on demand with no events. The daemon's real win is (a) a **warm** snapshot,
no su per read, and (b) **event-driven** observation via netlink (link/route/rule),
pushing invalidations instead of the app polling.

The hard engineering is not the vantage (always root netlink) but answering "is
**this** uid routed through a VPN": per-uid fib rules plus the table they point to,
split routing, and resync after missed events — a tunnel interface alone is not
proof. Account for initial resync and event-source failure. An LSPosed signal can be
an additional trigger but must not be the sole truth for native-only installs.
Temporary self-filter toggling was considered and rejected: it introduces
config/event races and changes the diagnostic environment.

**Netlink is not the whole observer: it triggers and measures membership, but it
cannot classify a VPN.** The kernel has no "VPN" flag on an interface — VPN-ness is an
Android framework concept (a `NetworkAgent` with `TRANSPORT_VPN`), not a link/route
property. Netlink gives interfaces, routes, rules and change events, so it answers
"did routing change" and "is this uid steered into table T"; it does **not** answer
"is interface X a VPN rather than, say, an IMS `ipsec` tunnel." That authoritative set
of VPN interfaces lives in the framework, reachable from root only via `dumpsys
connectivity` (fragile text) or a hook — and the per-uid membership check needs that
set as its input in the first place. So the core observer is netlink **plus** a
framework VPN-classification signal, not netlink alone. The existing app-side
`vhhelper observe app-vpn-state` (a typed `VpnOff`/`NotRouted`/`Routed`/`Unknown`
observation that parses `dumpsys` for the VPN set, then reuses the shared
`self_routed_for_interfaces` membership check, with an `ip route get … uid …` fallback
for unmanaged userspace tunnels) is a working prototype of that primitive; the core
observer should reuse it and replace only the polling with netlink-driven triggering,
not carry the `dumpsys` text-parse forward as the change detector.

## 9. IPC and access control

Agreed direction: a local UDS connection between the APK and the root service. The
existing kernel control/telemetry protocol (docs/protocol.md v2, frozen) is a
different boundary and is **not** replaced because core IPC changes; keep its codegen
and diff tests.

**Authentication is not the hard part, and there is no cryptography here.** The whole
system's security model is already DAC + SELinux + the root-manager grant — all app
state today is plaintext, gated only by file permissions and SELinux labels. The
daemon inherits exactly that. The one real shift is that a persistent daemon takes
authorization off the per-operation su gate (which today makes the manager vet the
package by uid + signature on every privileged write), so the boundary must be
re-established once:

- **Primary gate (capability via DAC):** at first setup the app, in its own
  su-session (the manager's one grant prompt), creates the control socket as a
  filesystem path, `chown`ed to its **own uid**, mode `0600`. Thereafter the kernel's
  ordinary file-permission check admits only that uid — the same mechanism that
  already protects the config files. One su per app start is still far cheaper than
  today's many su calls.
- **Second layer (SELinux):** the socket is a **filesystem path with its own SELinux
  label** (never an abstract-namespace socket, which cannot be path-hidden and always
  shows in `/proc/net/unix`), so `untrusted_app` is denied `connect()` by policy, the
  way it is already denied `/proc/net/route`. A dedicated app domain assigned by the
  manager on signature makes this a clean type rule.
- **Defense in depth:** the daemon may still take `SO_PEERCRED` on accept and map
  uid → package + signing cert via `packages.xml`, to fail closed (not admit a
  stranger) when a reinstall changes the uid.

An APK update must not let an untrusted binary become root service code; minimize
policy changes; no world-writable endpoint or arbitrary shell RPC. But none of this
needs tokens, per-message auth, or a protocol version invented for security.

The whole of "authentication under SELinux" therefore collapses into the one spike
that actually matters: **the socket/label/domain plumbing under enforcing SELinux**,
a per-device, per-manager plumbing question, not a security-theory question, and it is
the first spike. The eventual support matrix is Magisk / KSU / APatch, whose sepolicy
and seapp mechanisms differ — but the concept only needs to be proven on **one**
manager first. The available device runs a **custom KernelSU-Next**; its `ksud profile`
CLI (a root profile grants a package/uid its `capabilities`, a custom `selinux_domain`,
uid/gid/groups, and per-package sepolicy via `ksud profile set-sepolicy`) is a convenient
handle for the **spike**, but it is **not** the production substrate and core must not
depend on it — see the delivery/rights model in §9.3. Magisk and APatch are later
breadth-of-support work, not a prerequisite for go/no-go.

Detection surface is not a separate concern. Current self-hiding rests on SELinux +
platform (`untrusted_app` denied `/proc/net/*`, hidepid hides other processes from a
non-root app), **not** on SUSFS. A filesystem-path socket with a dedicated SELinux
label is detected no more easily than the kernel module already is. Do **not** make
invisibility depend on SUSFS — that regresses a universal SELinux property to
"whatever the kernel happens to support." And the adversary probes for the VPN, not
for the control plane.

**RPC library.** Narrow to **tarpc over UDS** unless a prototype shows a blocker: it
is Rust-defined request/reply plus `watch_state(after_revision)` long-poll, which is
exactly the traffic shape (§10). Drop tonic — its protobuf build step and HTTP/2
streaming solve a problem (§10 says there is no firehose) and contradict the project's
own already-made decision to drop the gobley/UniFFI codegen as not worth the
dependency. Rust-defined public types are the single source of truth; separate public
wire types from private implementation; core module version, APK version and backend
wire version must not be conflated (§11).

Prototype acceptance: an app-originated connection admitted by DAC + SELinux under
enforcing policy; request/reply and subscription; disconnect/restart and snapshot
recovery; slow client and bounded buffers; an operation surviving client loss; a
reinstall/UID change failing closed.

References checked during discussion:

- [tarpc](https://github.com/google/tarpc)
- [tarpc UDS](https://docs.rs/tarpc/latest/tarpc/serde_transport/unix/index.html)
- `SO_PEERCRED` / `struct ucred`, `SCM_RIGHTS` fd-passing, `seapp_contexts` (per-manager)

### 9.1 Two IPC boundaries: tarpc for the app, the control/telemetry wire for backends

tarpc/UDS is the **app ↔ core** boundary only. The **core ↔ backends** boundary is the
existing frozen `control` v2 / `telemetry` v1 wire (docs/protocol.md), which core — as the
activator (§4) — already speaks: it writes `control` v2 config to each backend and reads
`telemetry` v1 stats/status back. Two different problems, two different tools:

- **app ↔ core:** tarpc/core-rpc — typed edits, operation lifecycle, state subscription.
- **core ↔ backends** (kmod / KPM / built-in / **Zygisk**): protocol.md text — snapshot
  config out, telemetry in; compiles in freestanding kernel C and a minimal in-process hook,
  `cat`/`echo`-debuggable, drift-guarded by golden vectors.

Do **not** reach for tarpc on the backend side. An earlier sketch had the Zygisk companion be
a tarpc client of core; that was wrong — Zygisk is a backend, so its channel is protocol.md
like every other backend.

**Two version fuses on two boundaries, decoupled.** With core as the hub, `telemetry` v1 /
`control` v2 now guards *core ↔ backend* skew (its reader moved from the app to core), and
core-rpc's `PROTOCOL_VERSION` guards *app ↔ core*. The app is insulated from the backend wire
version — it reads stats through tarpc, not by parsing telemetry itself — so the kernel wire
can bump with the core module while the app's contract moves on its own schedule (exactly the
apart-versioning protocol.md already has, now landing on two physical boundaries).
protocol.md §9's "we have no hub" flips: v2 has a hub (core); telemetry flows
backend → core → app; the wire is unchanged, only the reader is new.

### 9.2 Zygisk file delivery and process boundaries

Zygisk hooks run inside the target app. Configuration remains a control-v2
snapshot in the module's `targets.txt`, read through the privileged module-dir
fd in `on_load`. The library is loaded afresh in each forked child and its cache
is per process. A newly launched target reads the updated file; an existing
process keeps its snapshot until restart. No zygote reboot is needed for that
existing behavior.

Core owns selection and publishes the file directly. Root-manager integration
continues to load the Zygisk library in the correct process/ABI. Core does not
inject the library, install app hooks from its own root context, or require the
VPN Hide APK to remain alive for publication. Actual hook measurements remain
in the app process. Publishing a file is evidence of delivery, while runtime
and per-process consumption require independent evidence.

The reserved empty companion is not required for this migration. Per-UID
statistics and an optional relay remain future work. If a companion is added,
its role is delivery/collection; core retains backend selection and canonical
configuration ownership. A future statistics channel needs its own opt-in,
off by default, and must preserve short-lived descriptors and the existing
process/SELinux boundaries. No companion, new hook RPC or statistics mechanism
is introduced as a condition of the main V2 result.

### 9.3 App-side client and cross-manager delivery/rights model

The Kotlin↔core client is **Rust (the tarpc UDS `CoreClient`) exposed to Kotlin through plain
UniFFI** — no hand-written wire in Kotlin (fragile), no Kotlin-Multiplatform gobley (broke on
AGP 9; we are Android-only). UniFFI's `async_runtime = "tokio"` turns each async Rust method
into a Kotlin `suspend fun`; the state subscription is a **suspend-long-poll** (`awaitNextState`
parks on the mirror's `watch` and resumes on change — no callbacks, no Stream→Flow). The domain
payloads cross the FFI as **JSON strings**, not per-type UniFFI records: the FFI surface stays
tiny (connect / hello / submit / state) and stable while the domain churns, and both ends already
`serde`/`kotlinx.serialization` the same shapes.

**Spike result — the FFI client end to end on device (KSU-Next Pixel 8 Pro, `crates/core-ffi` +
the app `dev` sandbox): GO.** The full chain — UniFFI + JNA + tokio + tarpc-over-UDS — comes up
inside the real `untrusted_app` process; the async methods complete across JNI as suspend funs;
a Replace bootstrap then a Toggle round-trip (`{"Ok":{"op":N}}`), optimistic concurrency holds
(`config_revision` 0→1→2), and the suspend-long-poll mirror delivers each change reactively while
the netlink observer feeds real routing into the same snapshot. Two Android-build notes: JNA needs
the cdylib **extracted on disk** (`useLegacyPackaging`/`extractNativeLibs`, else `Native.register`
can't dlopen it), and **R8 must not minify** the JNA runtime (`com.sun.jna.Native.fromNative` is
called from native code) — both scoped to the dev build here, and keep-rules or no-minify in prod.

**Delivery and rights are manager-agnostic — core does not depend on any one manager.** The
production core is a **Magisk-format module launched at boot** by a one-line `service.sh` exec of `vpnhide-core service`,
the same format Magisk / KSU / KSU-Next / APatch all run (exactly how v1 already ships its
backends). This splits cleanly by owner:

- **App→core SELinux** (`untrusted_app_XX` → the socket's `sock_file` write + `unix_stream_socket
  connectto` to core's domain) ships as the module's **`sepolicy.rule`**, consumed by every
  Magisk-compatible manager (`magiskpolicy` / `ksud sepolicy` / APatch). The runtime
  `ksud sepolicy patch` used in the spike is the throwaway equivalent.
- **Core's capabilities** come from the **boot-launch root context** (full caps, incl.
  `CAP_NET_ADMIN`) on every manager. The empty-caps gap seen in Spike A/B is an artifact of
  launching core via an *app/shell `su`* (which on KSU applies a RootProfile that drops caps),
  **not** of boot launch. `CAP_NET_ADMIN` is in any case **optional** — the observer only used it
  for `SO_RCVBUFFORCE` and already falls back to `SO_RCVBUF`.
- **Core's SELinux domain** for the `connectto` rule: simplest cross-manager option is to let core
  run in the manager's root domain and ship a short per-manager allow set (`magisk` / `ksu` / …);
  a custom `vpnhide_core` domain via `type_transition` is one rule but leans on custom-type support
  that varies by manager. The primary gate stays the **socket file label** (a path we own +
  `restorecon`), with `SO_PEERCRED` uid+package as fail-closed.

`ksud profile` (capabilities / `selinux_domain` / per-package `set-sepolicy`) is a genuine
convenience **for the spike** on the KSU-Next device, but is not required by, and must not be
depended on by, the production design.

## 10. Traffic and resource expectations

One initial snapshot, commands on committed user actions, and updates when relevant
facts change. Editor keystrokes stay local. Statistics subscriptions have a bounded
cadence and only run when needed; diagnostics and export are explicit heavier
workloads. Suspend app subscriptions when not useful; core still manages the system
without an app connection. An idle connection need not poll constantly.

No RPC per packet, syscall, hook invocation or app network callback. Hooks use
locally applied policy. Avoid whole-inventory refresh on each network transition. Use
bounded collection workers, deadlines, coalescing and resync after lost events. Large
bundles must not block small control messages or imply unbounded buffers.

## 11. Ports and module migration

Ports implementation already resides in `crates/activator/src/ports.rs` and
`lifecycle.rs`: UID policy projection, IPv4/IPv6 restore, OUTPUT jumps, boot apply and
uninstall cleanup. Move it into core's Ports implementation; remove the separate ZIP only
after migration ownership is explicit.

Current boot code waits for `bw_OUTPUT`, applies once, then repeats after 30s. A
persistent core can reconcile missing rules, but needs verified triggers or a bounded
inspection policy; persistence alone does not detect netd rule rebuilds. IPv4/IPv6
changes are not one atomic transaction — report partial outcomes honestly. Rules can
survive core failure; restart must inspect rather than assume absence.

**Dual-writer: refuse now, not remove later.** The most dangerous state is a
half-migrated device where two things believe they own activation — for a stealth
tool that is a silent leak. So the moment core is present, every other writer must
**fail closed immediately** (core holds an epoch/lock marker; legacy paths check it
and no-op with a log), not merely be deleted at the last step of §12. This is now
enforceable rather than conventional, because the config file is DAC output-only for
everyone but core (§4). Per boundary the cutover order is: install core → core
adopts existing state → legacy writer disabled — all before core's first write. Old
Ports uninstall must not erase rules core now owns; the built-in bridge similarly
disappears only after core owns its boot tasks. Existing KPM/kernel/Zygisk consumers
need compatible policy delivery throughout.

Plan common-module disable/uninstall, absent core, interrupted upgrades, rollback,
root-manager differences, APatch secrets, logging leases and full reset explicitly.
Core failure must be visible and must not be presented as disabled protection:
already-applied backend behavior can persist. Preserve a restricted maintenance path
for recovery without reintroducing independent ordinary writers.

## 12. Proposed incremental development sequence

1. Spike first, before committing to the daemon: (a) event-driven observation value —
   can a uid-0 netlink observer answer "is this uid on VPN" reliably (§8); (b) the
   socket/label/domain plumbing under enforcing SELinux, proven on KSU-Next first, the
   other managers later (§9). Both device parts run on the available KSU-Next phone.

   Spike A result (KSU-Next Pixel 8 Pro, `crates/netlink-spike`, driving Floppa VPN
   over adb): **GO.** A uid-0 `NETLINK_ROUTE` listener caught 4/4 VPN up/down edges
   promptly, each transition's ~135 messages coalesced by a 250 ms debounce into one
   clean edge, and the shared `self_routed_for_interfaces` primitive flipped the uid's
   membership true/false correctly per edge — no misses, no spurious edges. Confirmed
   the §8 caveat empirically: an IMS `ipsec20` interface is always name-matched into the
   VPN set, so the observer must cross-check the framework's `TRANSPORT_VPN` set (benign
   here only because the uid was not routed into `ipsec`'s table). The ENOBUFS path was
   then forced (shrunk `SO_RCVBUF` + slowed drain): the kernel signalled overflow, the
   observer resynced by re-dumping, and recovered the **correct current state** each time —
   proving the "events are change-notifications, not a log; resync recovers truth" rule
   (§6/§8). Overflow does not occur from real VPN toggles (a transition is only ~135
   messages, slow); it had to be forced. Incidental: KSU-Next gives uid 0 **without**
   `CAP_NET_ADMIN` (`SO_RCVBUFFORCE` denied), though plain netlink monitoring needs no cap.
   Still open (lower priority): Wi-Fi↔cellular handover (expect no vpn edge) and a
   rule-only edge (split change while the tunnel stays up).

   Spike B result (KSU-Next Pixel 8 Pro, `crates/uds-spike`, enforcing): **GO.** A root
   daemon in domain `ksu` bound a UDS *path* socket; a client run as `untrusted_app` (via
   `su -Z u:r:untrusted_app:s0:c512,c768`) was **denied by default**, then after three
   targeted live rules (`ksud sepolicy apply`: `allow untrusted_app ksu unix_stream_socket
   connectto` + `... shell_data_file sock_file write` + `... shell_data_file dir search`)
   **connected** under enforcing and the daemon read its `SO_PEERCRED`. A different,
   ungranted domain (`priv_app`) stayed denied. So the socket/label/domain plumbing works
   and a targeted grant opens exactly the app→daemon path. The §9 nuance was made concrete:
   granting the shared `untrusted_app` type opens the socket for **every** app, so real
   scoping to only vpnhide needs a dedicated `selinux_domain` (via `ksud profile`) and/or
   the daemon's `SO_PEERCRED` uid+package check — both confirmed available. (Sepolicy
   grants applied live are non-persistent; production ships them via the module's
   `sepolicy.rule`, §9.3.)

2. Stand up the tarpc-over-UDS daemon skeleton (publish / lifecycle / listener) and the
   app-side client; introduce the cross-process epoch (§6). **Done** on `dev-2.x`:
   `crates/core-{rpc,daemon}`, and the Kotlin↔core client validated end to end on device
   via `crates/core-ffi` (UniFFI, suspend-long-poll) against the isolated app `dev`
   sandbox — see the §9.3 FFI spike result. The app-side connection recipe (socket DAC +
   the two SELinux rules) is recorded there.
3. Implement root observations and consistent state subscription; integrate UI against
   the read-replica.
4. Transfer durable config operations and recovery, porting the config-operation
   reducer to Rust (§7) with the existing `ConfigCoordinator` behavioral tests as the
   language-neutral vector.
5. Absorb every activator into core — derive the kernel wire and Ports policy in-process
   and apply them directly (§4); make core the sole writer of the config file and its
   reader on restart; migrate installed module state.
6. Integrate app-context diagnostics and capture with core session provenance; the
   app-side generic reducer may remain Kotlin.
7. Remove obsolete activators, bridge/Ports packages, writers, caches and su
   orchestration.
8. Validate upgrade/restart/rollback and independent version combinations before 2.0.

Progress on `dev-2.x`: steps 2 (daemon skeleton + FFI client, §9.3) and the config-edit
reducer are done; the **durable config file** (part of step 5) is implemented —
`core-daemon/config_file.rs` is the disk codec between the polymorphic legacy on-disk JSON
and core's clean `CanonicalConfig` (a faithful port of `StorageConfig.kt`'s
`parseCanonicalConfig`/`canonicalConfigJson`, so LSPosed and the activator read a
core-written file identically). `CoreState::load` reads it on startup (restart recovery) and
`submit_edit` is durable-first (writes before acknowledging). The path is overridable via
`VPNHIDE_CONFIG_PATH` so the dev sandbox core never touches the real
`/data/system/vpnhide_config.json`.

The **operation lifecycle** (`operation.rs`) is the core-side port of the execution core of
the app's `reduceConfigOperation` — an accepted edit runs an ordered phase plan through a
`PhaseRunner` seam, each phase recording a real outcome into the `OperationRecord`, the first
non-confirmed phase stopping the plan (later phases stay `NotAttempted`), and only a confirmed
`Persist` committing the candidate (a later phase failing keeps what `Persist` wrote). The
app-only concerns migration leaves in Kotlin (drafts, UI-edit conflicts, bridge-source) are
**not** ported (§3/§7). Recovery (an `Unknown` outcome → readback → resolve/Held) is
deliberately deferred to step D: today the only real phase is `Persist` (a local atomic write,
never `Unknown`); the `PhaseRunner` seam is where the D activation runners plug in, and a fake
runner already drives every outcome in tests.

**Step D (backend activation) has started, kmod/built-in family, device-validated.**
`core-daemon/activation.rs` links the activator (additive pub surface on `vpnhide_activator`)
rather than shelling out to the per-backend binaries: it projects the config to the built-in
control-v2 wire (`project_native_for_family`, real `pm` uid resolution) and delivers it to the
control node (`deliver_kernel_config`, which refuses a node not reporting the expected
backend). The `Native` phase runner does project → deliver; `plan_for` adds `Native` after
`Persist` on any config change when a control node is configured. **Isolation:** the control
node is an overridable path (env `VPNHIDE_KMOD_CTL`) with **no default** — unset means no
native write at all, so the dev sandbox core cannot touch the real `/proc/vpnhide_ctl` (a
write there replaces the live backend's entire state). Validated on the KSU-Next device
**read-only**: core parsed the live telemetry-v1 status (`backend=0x4 kver=0x6019d
hooks=0x20003ff error=0x0`) and projected the real config into the exact control-v2 wire the
live backend runs (same `20003ff` mask, and every uid the live stats report appears in the
projected target set) — without writing. **Central backend detection** (`core-daemon/backend.rs`) is the point of core owning all
state: instead of each activator probing runtime identity and self-selecting, core makes one
observation — `NativeBackendStatus { active, installed }`, published in `StateSnapshot`.
`active` is the live kernel family from the control node's `backend` id (read-only, §2: runtime
identity, not a module dir merely existing); `installed` is module-dir presence. The Native
phase now projects for the **detected** family (the built-in hardcode is gone), and `selection.rs`
is subsumed. Device-validated read-only: `active=Some(Builtin) installed=[Builtin]`, and
`project` drives the same live wire through detection. Still to do for D: KPM and Zygisk
(companion) `active` liveness signals + central selection/loading policy; the Ports phase; the
sole-writer cutover; and recovery (now that a `Native` write can be `Unknown`).

**Step D continuation: direct kernel selection and loading (host-validated).**
`backend::ensure_kernel_backend` now owns the kernel-family decision used by the
Native phase. Its order is:

1. Adopt a compatible live built-in/kmod identity. This precedes artifact priority;
   a live built-in needs no `vpnhide_builtin` directory or bridge.
2. Only a definitely missing node permits selecting an enabled kmod artifact.
   Both `module.prop` and `vpnhide_kmod.ko` must exist; `disable` and `remove`
   markers exclude it. A malformed, unreadable, incompatible or foreign node
   stops selection, rather than being treated as absence.
3. Load that artifact directly through the shared Rust load executor, preserving
   its timeout and `filesystem_hiding` argument from the candidate config.
   The loader rechecks control-node absence and refuses a live KPM or an
   unobservable detected KernelPatch runtime. Failed `kpatch kpm list` is an
   observation error, not an empty module list.
4. Verify the selected kmod identity after loading, then project and deliver.
   Delivery still checks identity on the same open file that receives the write.
   A load failure or unexpected post-load owner stops the operation. No retry,
   unload, or fallback to another family is dispatched by this slice.

The existing native preference is `kmod > built-in > KPM > Zygisk`, subject to
adopting a live kernel owner first. This kernel-only checkpoint did not manage
KPM/Zygisk; the KPM continuation below extends the selector. Cross-family
fallback after a failed attempt remains disabled.
Core remains the selection owner; backend-specific Rust modules do not become
autonomous activators or perform neighbor selection over RPC.

Loading is explicitly enabled by `VPNHIDE_KERNEL_MODULES=<module-root>`, in
addition to `VPNHIDE_KMOD_CTL=<control-node>`. Without the former there is no
`insmod`; without the latter there is no Native phase. This preserves development
isolation while the common module and legacy-writer cutover are unfinished.
Do not enable the production load path alongside legacy boot writers. No module
scripts are changed or removed in this stage, and core still does not activate
on boot without a submitted edit. Native-only execution uses no APK measurement
or LSPosed service.

After an activation attempt core refreshes backend observation, independently of
whether the Native phase succeeded. Confirmed persistence survives a subsequent
load/apply failure. Durable operation recovery and write-outcome uncertainty are
still unfinished; these changes do not establish exactly-once effects or safe
production cutover.

Host tests cover the selection policy, unusable artifacts, wrong protocol/owner,
failed loading, and KPM presence/uncertainty. Subprocess tests exercise the real
RPC and phase runner with a fake `insmod`, including the loader arguments,
built-in operation without a bridge, post-load identity refusal, saved intent,
and refreshed observation after failure. A regular test file is not a proc node;
these tests do not validate Android loading, SELinux, QEMU, or device boot.
The existing Rust CI job now includes core-rpc/core-daemon checks and tests.

Validation: 106 host tests passed across activator, core-daemon and core-rpc;
the wider non-Zygisk host suite passed 155 tests. Host Clippy (`--all-targets`,
warnings denied) and library Clippy for Android ARM64/ARMv7 passed. Two Zygisk
`setsockopt_tests` fail identically in an unmodified `e4127ea8` worktree in this
environment. Workspace rustfmt also fails on that baseline (18 files); touched
Rust files are formatted. These are baseline limitations, not a claim of green
full CI.

**Step D continuation: KPM management from core (host-validated).**
With `VPNHIDE_KPM_LOCK` explicitly set, `backend::select_native` probes a single
authenticated KPM transport in addition to the kernel control node. It adopts
a compatible live kernel owner or KPM; simultaneous kernel/KPM identities are
an error. A loaded raw KPM needs no companion directory. With no active backend,
eligible kmod artifacts precede KPM artifacts; disabled/removed artifacts are
ineligible. Failed discovery, authentication, enumeration, load or identity
validation never triggers another backend's load.

The shared activator now exposes `CoreKpm`, a transport handle with an explicit
lock path. It reuses KPatch-Next CLI and APatch/FolkPatch direct-supercall code,
but does not select backends, read canonical config or invoke old activator bins.
Core supplies the selected artifact, filesystem load feature and KPM-family
projection. Loading retains the supported-kernel-family check and verifies live
telemetry after the runtime reports success. Optional filesystem loading emits
`filesystem_hiding=1` only when enabled, retaining the existing KPM ABI.

Status requires a complete telemetry-v1 response with the KPM identity. Before
dispatch, core checks capacity, config framing, readiness and the absence of a
kernel control owner again. Identity validation and config dispatch hold the same
KPM lock. A busy lock fails preflight immediately, while legacy lock behavior is
unchanged. The management lock must be `/data/adb/vpnhide_kpm/ctl.lock` in a future
production launch so legacy readers cooperate; tests use a private lock.

The core sends exactly one KPM config command per attempt, without the legacy
activator's generic retry loop. A post-dispatch error becomes `Unknown`, rather
than known failure. Confirmed persistence remains visible and duplicate operation
IDs with the same original base and payload query the existing record; reusing
an ID with a different request returns `IdReused`. New edits are blocked after Unknown in this core
instance. Durable records and restart fencing are implemented (see journal slice below);
evidence readback for Unknown is implemented for Persist and Zygisk publication;
kernel/KPM Native still stays Unknown until a reliable apply receipt exists.
Restart alone is **not** proof of completion and must not re-dispatch uncertain effects.
KPM remains opt-in, requiring `VPNHIDE_KPM_LOCK` and `VPNHIDE_KMOD_CTL`; artifact
loading also requires `VPNHIDE_KERNEL_MODULES`. Common-module boot and legacy
writer cutover are not enabled by this stage.

Initial observation completes before the socket admits commands, preventing a
first mutation from competing with startup for the KPM lock. Backend publication
refreshes after attempts, and the read-only `project` tool now honors the same
paths and KPM observation as the service.

Subprocess/RPC tests cover 22 KPM scenarios: raw active adoption, loading and
filesystem feature defaults, inactive/disabled artifacts, kernel priority and
conflicts, unsupported kernels, failed or identity-less loads, changed/foreign/
incompatible/truncated status, partial readiness, busy locks, failed enumeration,
single dispatch with an unknown result, deduplication and rejection of new edits
after uncertainty. These are host checks with fake runtime commands. Direct
APatch/FolkPatch supercalls, Android permissions and actual kernel loading have
not been executed by these tests.

Validation for the KPM continuation: 107 core/activator/RPC host tests passed;
the wider non-Zygisk suite passed 156 tests. Clippy with warnings denied passed
on the host and Android ARM64/ARMv7, including all targets. Modified Rust files
pass rustfmt. The previously reproduced workspace formatting and Zygisk baseline
failures remain outside this stage; full CI is not claimed green.

**Step D continuation: Zygisk policy publication (host-validated).**
`VPNHIDE_ZYGISK_MODULE` enables the verified file recipient. With no compatible
live kernel/KPM owner, eligible kmod artifacts precede manageable KPM artifacts,
then Zygisk. A KPM artifact without a runtime is ineligible; an authentication or
enumeration error is unavailable evidence and stops selection. A live KPM without
an enabled management channel also stops selection. Failed load attempts never
fall through to Zygisk. Opt-in remains required while legacy writers exist.

The Zygisk module advertises `id=vpnhide_zygisk` and `vpnhideControl=2` in its
root-managed manifest. Core requires that identity/profile, enabled installation
and at least one matching ARM64/ARMv7 ELF shared-library artifact. Legacy bundles
without the profile need an updated module before this development path can own
their policy. Artifact suitability is not proof of runtime loading or hook health.

Core pins the recipient directory, revalidates its inode/manifest/artifacts after
projection, and publishes through the pinned directory. Redirected destinations,
staging links, disabled/removed modules and replaced directories are refused.
The shared atomic writer publishes the existing `targets.txt` format/mode;
core also syncs the directory before confirming publication. Kernel/KPM runtime
facts are rechecked after the PackageManager wait, immediately before dispatch.
Publication errors
after the write boundary are Unknown. Removing all targets writes an empty
snapshot, rather than leaving the previous policy installed.

App/core RPC is now version 2 because `NativeBackendStatus.selected` records the
last backend prepared for a policy-apply attempt in this core epoch. The matching
operation record gives its outcome. `active` remains independently observed
runtime identity; neither an installed library nor a successful Zygisk file
publication sets it. No synthetic root measurement replaces the existing
app-context heartbeat/hook probes. The client adopts the updated JSON state,
and version-1 clients are rejected by the handshake. Backend control-v2 and
telemetry-v1 formats are unchanged. The `dump` tool includes selected state.

Host unit tests cover atomic replacement, recipient changes, redirects, invalid
profiles, protocol/framing checks and both ARM artifact classes. Nineteen real
core-process/RPC scenarios cover selection/opt-out, priority, unmanaged/unknown
KPM, incompatible modules, publication and last-target removal. These tests do
not run Zygisk injection or validate Android root-manager/SELinux behavior.

This stage publishes Zygisk policy and records the selection; it does not unload
old hooks or retire another installation's existing boot policy. Coordinated
handoff of legacy files/scripts and stale processes belongs to ownership transfer.

Validation: 164 non-Zygisk host tests passed, including 19 Zygisk publication
scenarios within the core RPC suite. Host Clippy (all targets, warnings denied)
passed with the FFI client included, as did Android ARM64/ARMv7 Clippy for the
core/activator targets. The existing Rust CI Clippy step now includes core-ffi.
Modified Rust files pass rustfmt. The previously recorded workspace-formatting
and native Zygisk host-test baseline failures remain; full CI/device validation
is not claimed complete.

**Durable operation journal + evidence readback (journal milestone).**
`core-daemon/journal.rs` is the private store (§4 tier 3), separate from the
LSPosed-facing config file. Path: `/data/adb/vpnhide/core/operations.json`
(overridable via `VPNHIDE_JOURNAL_PATH`; colocated beside `VPNHIDE_CONFIG_PATH`
in the sandbox). It persists operation IDs, original requests, the intended
`candidate` config, phase progress, outcomes, retired-id retention, and the
durable `config_revision` counter. Intent is synced before each dispatched
phase effect; restart fencing turns in-flight `Running` (and unfinished
`settled=None` work) into durable `Unknown` without replaying effects.

`core-daemon/recovery.rs` reconciles Unknown from durable evidence only:
Persist compares the config file to the stored candidate (match → Confirmed,
readable mismatch/absent → FailedKnown, unreadable → keep Unknown); Zygisk
Native may confirm when `targets.txt` equals the projected wire; kernel/KPM
Native has no reliable apply receipt and stays Unknown (Held). Same-ID retries
return the existing record; different content or retired IDs are `IdReused`.
Subprocess crash-injection (`VPNHIDE_TEST_CRASH_AFTER`) covers persist-intent,
persist, and native-intent boundaries.

**Step 2 ownership / conflicts (landed).** Accepted mutations run under
`spawn_blocking` so RPC cancel/socket loss cannot abandon them. A
`dispatch_fence` ticket claimed at admit must still match before each external
effect (`invalidate_dispatch` fences delayed predecessors). Field-level
optimistic concurrency (`config_edit::write_set` / `conflict_field`) lets
unrelated edits merge against fresh state while same-field and Replace/import
domains conflict; `field_revisions` are durable in the journal.
`ConfigEdit::RepairActivation` re-applies Native without rewriting canonical
config (persist intent ≠ runtime apply).

Still open before Ports / production cutover: supervised async worker pool with
descendant accounting, manual recheck RPC / Held UX, Ports evidence, kernel
Native apply receipts, Magisk packaging, APK cutover. Zygisk live-process
evidence remains distinct from file publication.

**Step 3 Ports into core (landed).** Core Ports phase (opt-in `VPNHIDE_PORTS`),
honest IPv4/IPv6 reports, restart chain inspection, `ports_owned` uninstall fence.

**Step 4 exclusive ownership gate (in progress / inventory landed).** See
[`core-2.0-ownership-inventory.md`](core-2.0-ownership-inventory.md): writer inventory,
`ConfigLoad` Absent≠Unsupported≠Invalid, ownership markers, legacy activator no-ops.
Production writes remain explicitly enabled only; APK config writer not yet fenced.

**Step 5 common Magisk module (scaffolding landed).** `core/module/` boots
`vpnhide-core` with adopt/journal/BOOT_APPLY; legacy service/post-fs scripts prefer
core; uninstall is restricted (no hook unload / no SuperKey touch). See
[`core-2.0-module-lifecycle.md`](core-2.0-module-lifecycle.md).

**Step 6 observations/freshness (landed).** `RoutingObservation` carries Current vs
Unknown vs last-good; framework VPN classification (dumpsys/env stub); backend
`installed`/`selected`/`applied`/`active` separation; lock-free backend refresh.
See [`core-2.0-observations.md`](core-2.0-observations.md).

Next: APK IPC cutover (step 7) consuming freshness/last-good; device smoke for
framework classify + reboot-without-APK.

Each boundary needs an explicit cutover. Do not run legacy and core writers in
parallel against the same canonical/runtime state. Port behavioral tests before
deleting them. Include killed service/worker, delayed dispatch, multiple clients,
same-field conflict, reset, slow subscriber, reinstall/UID change and backend
partial-failure scenarios. Test built-in first on the available device, then supported
kmod/KPM/Zygisk/root-manager combinations; one device cannot establish the whole
support matrix.

## 13. Independent near-term 1.x fix

The immediate bug is independent of core and must not wait for it: the Dashboard can
miss VPN-up because a correctly hidden network transition does not necessarily produce
a callback for the app's own UID. A foreground-bounded re-read of the real routing
state covers it.

Source inspection on 2026-09-17 (main `b8f4df41`, plus the UI-situation work):

- `startup/MainActivity.kt` calls `RoutingGateCache.refreshIfStale()` on `ON_RESUME`.
- `diagnostics/RoutingGateCache.kt` uses a ~4000 ms throttle since the last load; it
  is not a periodic timer and is a no-op before initialization.
- `diagnostics/VpnTransportWatcher.kt` registers VPN/default callbacks and debounces
  refresh triggers; no periodic VPN poll exists in these paths.
- The observation layer now carries a `ReadReason` (`Background` / `Transition` /
  `Explicit`) and `StaleMark`, and `ContextStateCache.refreshRetained(reason)` exists
  for a process-side trigger with no Context of its own — the foreground VPN-state
  poller is being wired on this machinery.

Polling must be foreground-bounded, single-flight and independent of suppressed
callbacks; use the minimal fresh routing measurement, not a full suite on every tick.
Unknown is not VPN-off. Preserve retry/quarantine/resource bounds.

Callback hiding has a separate invariant: if the app-visible network and properties do
not change, neither VPN-up nor VPN-down should produce a spurious lifecycle
transition. Preserve actual Wi-Fi/cellular, offline/recovery and initial delivery.
Track each registration independently; inspect capabilities/link-properties duplicates
as well as Available/Lost.

Device acceptance must include VPN on/off over unchanged Wi-Fi, Wi-Fi/cellular,
offline/recovery, multiple registrations, background/foreground and manual Retry.
Install the matching APK and reboot for system_server hooks before testing.

## 14. Existing code and contracts this builds on

The coordinators named throughout are the starting point, not something to design:

- `ConfigCoordinator` / `ConfigCoordinatorIo` / `ConfigCoordinatorView` — the config
  actor, its IO port, and the read-replica view.
- `ObservationCoordinator` / `reduceObservation` / `ObservationState` / `StateCache` /
  `ContextStateCache` / `ProjectedStateFlow` — the observation state machine and its
  projections.
- `CanonicalConfigRepository` / `RootSnapshotCache` — the app-side canonical writer and
  the su-snapshot root observation to be replaced by core ownership and a warm feed.

Contracts to preserve and update when ownership actually changes (this record alone
does not supersede them):

- [Config coordinator](../config-coordinator.md)
- [Observation coordinator](../observation-coordinator.md)
- [Mutation transport and descendant guarantees](https://github.com/okhsunrog/vpnhide/blob/main/docs/root-mutation-transport.md)
- [State transitions](app-state-transitions.md)
- [Diagnostic semantics](diagnostics-state-analysis.md)
- [Storage and activation](../storage.md)
- [Persistent paths and ownership](../state.md)
- [Backend protocol](../protocol.md)
- [Capacity/resource limits](../limits.md)
- [Diagnostics](../diagnostics.md)
- [Debug bundle](../debug-bundle.md)

**Hard cutover: production backend lanes on.** With the activators, the
built-in companion and the Ports module deleted, nothing else loads or
configures a backend, so the common module's `service.sh` now exports
`VPNHIDE_KERNEL_MODULES=/data/adb/modules`, `VPNHIDE_KPM_LOCK=/data/adb/vpnhide_kpm/ctl.lock`
(the lock every KPM reader shares) and `VPNHIDE_ZYGISK_MODULE=/data/adb/modules/vpnhide_zygisk`;
the scaffold check requires them. Selection policy is unchanged: a KPM
authentication or enumeration error still stops selection rather than risk
loading a kmod beside a live KPM. Diagnostics read KPM state through core's
own `vpnhide-core observe kpm-state` (status reply plus stats pages), and the
dashboard checks each backend module for the payload core loads
(`<kind>_artifact_state`) instead of an activator.
