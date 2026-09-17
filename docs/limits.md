# Capacity limits

Every hard ceiling on the runtime path: where it lives, what it is measured at,
what binds first, and what raising it would cost. Read this before changing
`MAX_TARGET_UIDS`, the wire format, or a backend's static arrays — several of
these ceilings are in third-party code we cannot move, and the interesting ones
are not where people expect.

Numbers here are **measured**, not estimated; see [Re-measuring](#re-measuring)
so they can be checked rather than trusted.

## Two paths, two ceilings

The single most common mistake is treating these as one budget:

| | control (`config`) | telemetry (`stats`, `status`) |
|---|---|---|
| direction | activator → backend | backend → app |
| bounds | how many apps can be **protected** | how many apps **report counters in one read** |
| overflow costs | apps left unprotected | numbers missing from a screen |

They are separate protocols with separate versions ([protocol.md §3](protocol.md)).
A telemetry ceiling cannot limit how many apps are hidden, and vice versa. "We
can't raise the app count because stats won't fit" conflates the two.

## Control — how many apps can be protected

| constraint | where | value | binds at |
|---|---|---|---|
| `MAX_TARGET_UIDS` | `crates/protocol/src/lib.rs`, mirrored in both backends | 160 | **160 targets — currently binding** |
| `KPM_ARGS_LEN` | `kmod/third_party/KernelPatch/kernel/include/kpmodule.h` | 1024 B | ~190 targets sharing one mask; fewer with many distinct masks |
| `ctl_write` payload cap | `kmod/vpnhide_kmod.c` (`count > PAGE_SIZE`) | 4096 B | ~800 targets |
| config parse snapshot | `.ko` heap; KPM serialized static scratch | 8 B/target | does not grow either kernel stack |

Measured control payloads under the v2 grammar (one shared full kernel
hookmask, five-digit UIDs):

| targets | bytes | KPM (1024) | kmod (4096) |
|---|---|---|---|
| 40 | 248 | ok | ok |
| 64 | 368 | ok | ok |
| 64, half in a work profile | 400 | ok | ok |
| 120 | 648 | ok | ok |
| 160 | 848 | ok | ok |
| 200 | 1048 | **over** | ok |

Two things follow.

**`KPM_ARGS_LEN` is not ours.** It is a fixed buffer in KernelPatch, baked into
the user's patched boot image, and `compat_strncpy_from_user` truncates into it
*silently*. Before the v2 grammar a 64-target config was ~1.6 KB and so could
not be delivered at all — it was cut around 40 targets, mid-line, with no
signal. Grouping by hookmask and dropping the `0x` prefix cut that to 368 bytes;
the mandatory `end <count>` record turned any remaining overflow into a loud
whole-payload rejection. See [protocol.md §4.3](protocol.md).

**The cap counts UIDs, not apps.** `resolver.uids_for()` returns every UID a
package has across profiles, so an app present in a work profile spends two
slots. The picker allows 159 selected-app UIDs and reserves the final slot for
VPN Hide's main-profile UID. A user selecting apps present in both the main and
work profiles therefore reaches the picker limit at around 79 apps.

## Telemetry — how many apps report counters

There is no fixed output app count. A uid's text cost depends on how many hooks
fired for it and how large its cumulative counters grew. Representative
single-record sizes from the real formatter, for 5-digit uids:

| profile | bytes/uid | approximate rows in one 4096-byte KPM page |
|---|---|---|
| 2 hooks, counts < 256 | 25 | 160 |
| 4 hooks, counts < 65k | 51 | 79 |
| 8 hooks, counts ~1e6 | 103 | 39 |
| all 12 KPM hooks, saturated u64 | ~285 | ~14 |

The two backends are not in the same position:

- **KPM** — each ctl0 reply remains capped at 4096 B by the module and both
  clients. The backend pages by ascending UID; the activator validates and
  aggregates every page before exposing one normal telemetry block to the app.
- **kmod** — `/proc/vpnhide_ctl` is a real `seq_operations` stream with one UID
  per record. There is no whole-output buffer or formatter ceiling.

KPM keeps no serialisation snapshot. Its live table stores only the 11 shared
kernel hook counters plus its optional filesystem hook rather than the 28-id global registry, and a zero UID
is the hash-table empty sentinel, so no separate `stats_used` array is needed.
At the current 160-target cap, including the static config parse scratch, the
resulting `.bss` is **19080 B**. That is still below the **23120 B** used by the
old 64-target layout before compact storage and cursor output.

## Raising a ceiling — options and cost

Ordered by cost. None of these are scheduled; this is the menu.

### Control

1. **The shipped cap is 160.** Compact 12-hook stats storage and cursor output
   keep KPM `.bss` at 19080 B, and config parsing uses serialized static scratch
   rather than growing the KPM stack. A typical one-mask, five-digit-UID wire is
   848 B and fits the KPM transport; maximum-width UIDs or many distinct masks
   may hit the independent byte limit sooner. The activator validates the exact
   formatted size and rejects the whole KPM update rather than sending a
   truncated target set.
2. **Past ~190 targets on the KPM**, the 1024-byte transport is the wall. The
   only way through is chunking — `config-begin` / `config-chunk` /
   `config-commit` across several ctl0 calls — which is a control-protocol
   change. The `.ko` needs none of this; raising its `PAGE_SIZE` check is a
   one-liner.
3. **Whitelist mode** makes the whole question moot for the kernel backends:
   with a non-zero `default` the enumerated set becomes the exception list, a
   handful of entries, and every ceiling above stops binding. The wire already
   carries the mechanism (`default <hookmask>`); nothing emits it yet. Zygisk
   cannot honour the inverted meaning without injecting into every app process,
   so its activator rejects a non-zero default. See issue #248 for the design
   discussion and the reasons it should be opt-in rather than default.

### Telemetry

1. **Bare hex in a future telemetry v2** — the same trick control v2 used:
   ` 0x3:0xf4240` → ` 3:f4240`, four bytes per cell, roughly +45% uids per read
   at the heavy profile. Cheap in code, expensive in delivery: telemetry's
   reader is the app, so bumping it means shipping the APK in step with every
   module. Only worth bundling with some other telemetry change.
2. **Cap by policy, not by configuration** — emit the top-N uids by total and
   mark that it happened. The user gets numbers for the apps that did something
   and configures nothing.

Explicitly **not** recommended: a user-facing "collect stats for these apps"
list. It makes the user maintain a second selection whose mistakes are
invisible — no numbers looks identical to nothing happened — to solve a problem
that is telemetry-only and already degrades visibly.

## User-visible overflow reporting

Target-set overflow is surfaced after Save: the activator emits a stable
`vpnhide-warning native_target_cap` marker with the resolved UID totals, the app
captures activator stderr, and the picker shows a localized long-duration
warning instead of the normal success message. The canonical package selection
is still saved in full; the warning describes the capped native runtime
projection.

## App mutation transport

These are configured allocation/input bounds, separate from the backend capacity
measurements above. See [transport protocol and recovery](root-mutation-transport.md).

| Resource | Enforced bound | Owner |
|---|---|---|
| UTF-8 command script | 2 MiB plus at most 64 B framing | `root_transport/input.rs`, Kotlin request adapter |
| Canonical readback | 2 MiB | `root_transport/mod.rs` |
| Latest receipt metadata | 4 KiB | `root_transport/state.rs` |
| Retained helper stdout | 4 MiB | `RootProcessRunner` |
| Outstanding process/pipe owners | 3 per retained runner | `RootProcessRunner` |

The canonical JSON is embedded as a JSON string in the helper response, so escaping
and the receipt consume part of the stdout budget. Existing persistence builders
also base64-encode canonical data inside the script, reducing the usable canonical
write size below the script's 2 MiB limit. Overflow reports unavailable/rejection;
it never falls back to an empty config or silently truncates a mutation. These
bounds do not raise or replace the 160-UID native target limit.

## App observation reads

The batched root snapshot retains at most **16 MiB of stdout**. Overflow drains
the remaining output but rejects the entire snapshot; it never publishes truncated
sections. Stderr is drained without retention. The shell deadline is 10 seconds;
the observation coordinator reports failure at 15 seconds if the worker has not
returned. It holds the single root-read lane until the launcher process and its
pipe readers finish. These bounds are separate from the mutation runner's 4 MiB
reply and three retained owners.

Other observation deadlines are 60 seconds, except Dashboard (120 seconds while
its loader still awaits the diagnostic suite). A read waiter can follow
superseded generations for at most twice its cache deadline. This bounds waiting,
not the lifetime of an uncooperative worker. See
[observation coordinator](observation-coordinator.md) for quarantine and retry rules.

## Diagnostic runs

One diagnostic run has a **120-second** whole-run deadline armed at admission
(`DIAGNOSTIC_RUN_DEADLINE_MS`): it covers the eligibility read, both probe phases
and the end-context read. Expiry finishes the run as not started or interrupted
and, if probes were already launched, begins a separate **30-second** drain
(`DIAGNOSTIC_DRAIN_DEADLINE_MS`). Draining joins the run's outstanding helper
jobs; it cannot interrupt a blocking `su` probe. If the drain deadline passes
first, the probe resource is quarantined and every new request is rejected with
`ResourceUnavailable` until the late helper actually returns. The coordinator
retains raw evidence for at most the latest attempt and the latest complete
measurement, plus the results of the eight most recent finished attempts for
already-issued handles. See [app state transitions](notes/app-state-transitions.md) §7.

## Re-measuring

Nothing here should be trusted because it is written down. The representative
control-side figure is guarded by a test — `crates/protocol` asserts that all
`MAX_TARGET_UIDS` ordinary Android app UIDs, using one shared full mask, fit in
`KPM_ARGS_LEN` with room for its trailing NUL. Maximum-width UIDs and arbitrary
per-app masks can cost more bytes, so the activator also checks the formatted
wire's actual length before KPM delivery.

The telemetry table is not guarded, because its input is a usage profile rather
than a constant. To redo it, format `n` uids × `k` hooks with
`vpnhide_format_stats` from `kmod/shared/vpnhide_logic.h` (a dozen lines of host
C, built with `gcc -I kmod`) and divide by the buffer under test. The KPM `.bss`
figure comes from `llvm-size --format=sysv kmod/vpnhide.kpm` after
`python3 kmod/kpm/build.py`.
