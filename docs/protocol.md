# vpnhide — control & stats protocol

Single source of truth for the wire format exchanged between the app and every
backend (`.ko`, KPM, Zygisk, LSPosed), and for the architecture that carries it.
This document is the **arbiter**: when two implementations disagree, this file
decides who is wrong. Every implementation (C / Rust / Kotlin) and every test
vector references it.

Status: `version 1` **frozen** — all seven OPEN items (§10) are resolved.
Being implemented on `feat/control-protocol`. Key shape: one always-on Java
(LSPosed) layer + exactly one active native backend, priority `kmod > KPM >
Zygisk` (§1.5); three `kind`s — `config` (in), `stats` + `status` (out, §4.3);
UID is the key on every channel incl. Zygisk; `u64` cumulative counters; `debug`
folded into config; the `.ko` channel is one folded node `/proc/vpnhide_ctl`
(write=config, read=status+stats); KPM uses the `kpm ctl0` supercall (§7).
Threat model: an **unprivileged** app — root-level detection is out of scope.

> **Storage & activation live in [storage.md](storage.md).** This document is the
> frozen **wire** spec (the bytes exchanged at runtime). The layer above it — a
> single JSON canonical on disk, the activator that derives this wire for the
> native backends, LSPosed reading the JSON directly (it does **not** consume this
> wire), and the APatch superkey — is specified there, and supersedes the
> storage-related statements in §1.3–§1.4, §6, and §7.4 below. The wire format
> (§4–§5) is unchanged.

---

## 1. Architecture

### 1.1 No hub — app is the orchestrator

There is no resident root daemon. The app (Kotlin) is the only orchestrator: it
serialises config snapshots into every channel via `su`, and reads stats back
the same way. Each backend reads *its own* channel independently. There is no
intermediary process between the app and the backends.

Rationale: the exchange is rare (per user action) and pull-based; a resident
root process would add a named, enumerable component on a device where node
visibility is deliberately minimised, and it would not remove the unavoidable
in-`system_server` Kotlin reader anyway (see §3). A hub buys nothing here and
costs stealth + lifecycle complexity.

### 1.2 State is per-backend; the protocol is shared

Two orthogonal axes, kept separate:

- **State is NOT unified.** Each backend owns its own channel/file. Backends do
  not know about each other. One writer per file, atomic replace
  (`tmp` + `rename`). This is what prevents races and torn reads, and keeps each
  SELinux label scoped to a single reader domain. Merging state into one file
  would mean multi-writer across three domains and one label that must satisfy
  every reader — rejected.
- **The protocol IS unified.** One grammar, one parser core per language, one
  codegen'd hook registry. Format is not a shared resource, so unifying it
  creates no races. This is the *only* axis we unify.

**One kernel backend at a time.** The `.ko` and KPM are alternatives, not
co-residents: they wrap the *same* kernel functions
(`rtnl_fill_ifinfo`, `inet6_fill_ifaddr`, `fib_route_seq_show`, …), and running
both at once hard-froze a device (kretprobe + KernelPatch inline hook on one
symbol, deadlocking the netlink path on the first target enumeration — observed
on a Pixel 8 Pro, 6.1). So at init each kernel backend MUST detect the other and
refuse: the KPM resolves a `.ko` marker via kallsyms (the `.ko` is a real module;
the reverse uses the KPM's `/proc` marker), and a refusal is surfaced as
`status.error = conflicting_backend` (§4.3/§5.1) — a readable state, not a silent
no-op. This does not weaken §1.2's "backends don't know about each other" for
*state*: it is a one-bit liveness check at load, not shared runtime state.

### 1.3 Channels

Every channel carries the **same text snapshot format** (§4). They differ only
in transport and in which records they carry (profiles, §6).

| Channel | Write transport | Read transport | Reader runs in |
|---|---|---|---|
| app ↔ `.ko` | `echo > /proc/vpnhide_ctl` | `cat /proc/vpnhide_ctl` (seq_file) | kernel |
| app ↔ KPM | supercall `ctl0` (`args`) | supercall `ctl0` (`out_msg`) | kernel |
| app ↔ Zygisk | `targets.txt` via module dir-fd | n/a (stats via §7 if added) | Zygisk process (zygote-forked) |
| app ↔ LSPosed | `/data/system/vpnhide_*` files | same files | `system_server` |

Channel-specific notes:

- **Zygisk** reads through the root-privileged module-directory fd Zygisk hands
  the module *before* app specialisation (`openat(dir_fd, "targets.txt", …)`),
  which bypasses SELinux. Read on every app launch; edits picked up on next
  force-stop + relaunch. No companion, no socket.
- **LSPosed** has no privileged endpoint with the app. The only shared medium is
  the filesystem: app writes via `su`, the hook in `system_server` watches via
  `FileObserver` (inotify) and reloads. Files are `root:system`, mode `0640`,
  label `system_data_file` — `system_server` reads via group `system`,
  `untrusted_app` falls to "other" and gets EACCES.
- **KPM** is the only true request/response channel (no file, no node) — see §5.

### 1.4 Where protocol code lives

No hub ⇒ the serialiser lives in the app (Kotlin). Three languages total, by
process:

| Language | Process | Role in protocol |
|---|---|---|
| C | kernel (`.ko` + KPM) | parse config; emit stats. Freestanding, `shared/vpnhide_logic.h`. |
| Rust | Zygisk process | parse its config profile (`targets.txt`). |
| Kotlin | app | **serialise** all config snapshots into every channel; **parse** stats readback. The "thick" end. |
| Kotlin | `system_server` (LSPosed hook) | parse its config profile from its file. Not removable — cannot inject a `.so` into `system_server`. |

The UI itself originates *data* (package→UID via PackageManager, per-app hook
selection) but the app is also the orchestrator, so wire serialisation lives in
Kotlin here. Parser parity across C / Rust / Kotlin is held by golden vectors +
differential testing (§8), not by shared code.

### 1.5 Backend selection: one Java layer + exactly one active native

Backends split into two layers that run *together*:

- **Java layer — LSPosed, always active.** It hooks Java APIs inside
  `system_server` (PackageManager, ConnectivityManager, …) — a vantage no native
  backend has. It is orthogonal to the native layer and is never "the one of N".
- **Native layer — exactly one of {`.ko` kmod, KPM, Zygisk} is active.** They
  cover the same native surface (netlink / proc / libc), so running two is at
  best redundant double-hiding and at worst fatal: the `.ko` (kretprobe) and KPM
  (KernelPatch inline) wrap the *same* kernel functions and deadlocked a device
  when both fired (§1.2). So the product runs one native backend, period.

**Selection is by fixed priority `kmod > KPM > Zygisk`,** among the backends
actually *loaded* (read from each backend's `status`, §4.3). The app:

1. reads `status` from every native backend that is present;
2. picks the highest-priority loaded one as **active**, writes it the real config
   snapshot, and writes the others an **empty** snapshot (no targets ⇒ hooks
   installed but idle);
3. if more than one native backend is loaded, **warns the user** (a second loaded
   backend is a misconfiguration to clean up, not a supported mode).

Two backstops make this safe even if the app's policy is wrong or not yet run:

- **Kernel guard.** A native kernel backend refuses to *install hooks* if the
  other kernel backend is already present (KPM checks for the `.ko` via kallsyms;
  the `.ko` checks the KPM's `/proc` marker) and reports
  `status.error = conflicting_backend` (§5.1). This prevents the deadlock at the
  source, independent of userspace.
- **No self-activation of dangerous overlap.** Boot scripts may *load* a backend,
  but the single-active decision is the app's; a loaded-but-unconfigured native
  backend filters nothing.

This does not contradict §1.1 (no hub) or §1.2 (per-backend state): selection is
a read-only policy the app computes from `status`, not a shared runtime store.

---

## 2. Design constraints that shaped the format

These are *why* the format is what it is. Do not "simplify" against them.

- **One consumer is freestanding kernel C** (KPM, `-nostdinc`, no allocator, no
  libc). This rules out protobuf/flatbuffers/serde runtimes and JSON-in-kernel.
  Text with a hand-written parser is not a compromise — it is the only thing
  that compiles everywhere.
- **KPM delivery is a CLI that takes a string and returns a string.** A binary
  format would have to be hex/base64-wrapped through argv anyway. Text matches
  the transport.
- **Debugging is by an LLM agent reading/writing raw** over `adb` (`cat`/`echo`).
  The format must survive shell quoting (no `"`, `$`, backtick, `\` in payload)
  and be self-describing enough to read without external context. This is why
  keywords are words, not magic numbers.
- **Pull only.** Kernel aggregates per-(uid, hook) counters; userspace reads
  periodically. No push, no per-hit events on the hot path.

---

## 3. Semantics

- **Snapshot, not deltas.** One write = the entire desired state. The reader
  replaces its state wholesale. There is no add/remove/clear. This makes state a
  pure function of the last write — it never depends on history, which removes
  the whole class of "partially-applied" / "stale leftover" bugs. It also matches
  `echo > node` (truncating redirect) and `FileObserver` ("file changed",
  inotify gives no deltas).
- **Mandatory header gating.** A payload without a valid header (§4.2) is
  rejected *whole* — a stray `echo 'debug 0' > node` does not silently wipe
  state, it errors. "Valid full state, or a loud refusal, never silent-partial."
- **Version gates compatibility.** A reader knows only its own version. A payload
  whose version is greater than known is rejected whole (not parsed). This turns
  any drift that slips past tests into a loud refusal instead of a silent
  misparse — exactly what is wanted across the 7-KMI matrix where
  userspace↔kernel skew is inevitable.

---

## 4. Wire format

### 4.1 Lexical rules

- **Encoding:** ASCII only, bytes `0x20`–`0x7E` plus `\n`. Any other byte in a
  line → reject that line.
- **Line separator:** `\n` only. On read, a trailing `\r` (CRLF) is stripped. On
  write, never emit `\r`. A trailing `\n` at end of payload is optional on read
  (but see §5 for its meaning under KPM truncation).
- **Whitespace:** one or more spaces/tabs between tokens is a single separator.
  On write, emit exactly one space.
- **Comments:** a line whose first non-whitespace character is `#` is ignored
  whole. **No inline comments** after data — keeps the parser from scanning for
  `#` mid-line.
- **Blank lines:** ignored.
- **Line order:** not guaranteed; readers must not depend on it.

### 4.2 Header

The first significant (non-blank, non-comment) line MUST be:

```
vpnhide <version> <kind>
```

- `vpnhide` — literal magic.
- `<version>` — decimal integer (this line is parsed specially; the hex rule in
  §4.4 does not apply to it). Current: `1`.
- `<kind>` — `config` (app → backend), `stats` (backend → app), or `status`
  (backend → app: module health + errors, §4.3).

A payload without this header, or with `version` > the reader's known version,
is rejected whole (§3).

### 4.3 Records

**config** (`kind = config`):

```
debug <flag>
target <uid> <hookmask>
```

- `debug <flag>` — `flag` is the literal `0` or `1`. At most one `debug` line;
  absent ⇒ unchanged-from-default (define default as `0`).
- `target <uid> <hookmask>` — one per target app. `uid` is the app UID;
  `hookmask` is a hex bitset of enabled hooks (bit N ⇔ `hook_id` N, §5/registry).
  Duplicate `uid` ⇒ last-wins (producer guarantees uniqueness; consumer defends).
  **UID is the key for *every* backend**, including Zygisk: the kernel backends
  have no PackageManager so they cannot key on package names, and Zygisk (which
  runs in the target's own process and could match either way) keys on `getuid()`
  for one grammar across all channels. The package→UID resolution is the
  producer's job (app / boot script), the same for all backends.

**stats** (`kind = stats`):

```
<uid> <hook_id>:<count> <hook_id>:<count> ...
```

- One line per uid that has any non-zero counter. Sparse: only non-zero
  `hook_id:count` pairs are emitted. `hook_id` and `count` are hex; `count` is a
  `u64` **cumulative since the backend loaded** (OPEN-3) — reads never reset it,
  so two readers don't race and the app computes deltas itself.

Asymmetry is deliberate: config uses a dense mask (a *set* over a small fixed
universe, applied in the kernel in O(1)); stats uses sparse `id:count` (most
counters are zero, and adding a hook shifts no columns — an old reader simply
skips an unknown `id`).

**status** (`kind = status`) — backend health, read the same way as stats:

```
backend <id>
kver <kver>
hooks <installed_mask>
error <code>
```

- `backend <id>` — which backend answered (registry ids: `0x0` .ko, `0x1` KPM,
  `0x2` Zygisk, `0x3` LSPosed).
- `kver <kver>` — running kernel version as the backend sees it (KernelPatch
  `kver`, e.g. `0x6019d`); `0x0` for non-kernel backends.
- `hooks <installed_mask>` — bitset of hooks that *actually* installed, in the
  same id space as a config `hookmask`. Lets the app show "requested vs active".
- `error <code>` — `0x0` = healthy; non-zero is the single dominant fault code
  (§5.1). At most one `error` line.

`status` exists because a kernel backend can refuse or only partially install for
reasons the app cannot otherwise see (no node, no logs unless `debug`). It turns
those into a readable state instead of a silent no-op — most importantly the
mutually-exclusive-backend case (§1.2): when the KPM detects the `.ko` (or vice
versa) it refuses and reports `error = conflicting_backend`, which the app
surfaces as "disable the other backend".

### 4.4 Numeric primitive

One primitive for every data field (`uid`, `hookmask`, `hook_id`, `count`):

```
0x  followed by one or more hex digits, any case on read
```

- `0x` prefix is **mandatory** on data fields. (If it is ever dropped it must be
  *forbidden*, never *optional* — an optional prefix = two valid spellings of one
  number = a drift seam. See OPEN-1.)
- Read accepts any case (`0xFF` == `0xff` == `0xFf`); **write always emits
  lowercase**. Liberal-in / strict-out: producers are deterministic, the consumer
  never trips on case. (Pin this with a vector — it is a classic silent-drift
  corner: C ASCII-folds, Rust forgets, they diverge on `0xC` vs `0xc`.)
- **Widths:** `uid`, `hookmask`, `hook_id` are `u32`; `count` (stats) is `u64`
  (OPEN-3). A value that overflows its field's width → reject the line (not
  saturate, not wrap).

The `debug` flag (`0`/`1`) and the header `version` are the only non-hex tokens,
and both are special-cased to their keyword/line.

### 4.5 Forward compatibility

- **Unknown first token** (keyword/record type) → skip that line, do not fail the
  payload. Lets a newer producer add record types without breaking older readers.
- **Unknown `hook_id`** → the app keeps it as "unknown hook N" when reading
  stats; the kernel masks unknown bits when applying a config mask
  (`mask & known_hooks`). New hooks never misfire on an old backend.
- **Empty payload after the header** (zero records) is valid.

### 4.6 Example

config (app → kernel):

```
vpnhide 1 config
debug 0
target 0x27fa 0x3ff
target 0x2947 0x004
```

stats (kernel → app):

```
vpnhide 1 stats
0x27fa 0x0:0x5 0x3:0xc 0x9:0x1
0x2947 0x2:0x1
```

---

## 5. Hook registry (global ID space)

The shared vocabulary between config masks and stats IDs. **Global ID space:**
one `hook_id` = one specific hook in one backend; numbering is project-wide;
bit N in a mask ⇔ `hook_id` N in stats. Adding a hook to any backend takes the
next free global id.

This is the **only** thing that is codegen'd. The wire grammar/parser is
hand-written (trivial); the registry is data referenced from five places
(`.ko`, KPM, Zygisk, app, `system_server` reader + UI labels), so id drift would
turn "hooks for app X" into the wrong hooks on another backend. One TOML →
codegen emits:

- C `enum` + names (kernel labels its counters),
- Rust `enum` + the per-backend "which bits are mine" mask,
- Kotlin `enum` (per-app picker UI + the `system_server` reader).

Registry source row (illustrative):

```toml
[[hook]]
id = 0
name = "fib_route_seq_show"
backend = "kernel"
```

Each backend applies `mask & own_hooks` and ignores foreign bits. A config mask
is global; a backend only acts on its own bits.

Current kernel hooks (`.ko` / KPM, 10): `fib_route_seq_show`,
`ipv6_route_seq_show`, `rtnl_fill_ifinfo`, `inet_fill_ifaddr`,
`inet6_fill_ifaddr`, `dev_ioctl`, `sock_ioctl`, `fib_dump_info`, `rt6_fill_node`,
`fib_nl_fill_rule`. Current LSPosed Java hooks (8): `lsposed_link_properties`,
`lsposed_network_capabilities`, `lsposed_network_info`, `lsposed_network`,
`lsposed_connectivity_result`, `lsposed_connectivity_callback`,
`lsposed_connectivity_network`, `lsposed_package_visibility`. Zygisk libc hooks
take the next free global ids when they are wired into per-hook protocol stats.

### 5.1 Error codes (`status`)

A small fixed enum, codegen'd from the same registry TOML as the hooks, so the
numbers are one source of truth across C / Rust / Kotlin (a status reader must
not invent its own codes). The `error` field of a `status` payload (§4.3) is one
of:

| code | name | meaning |
|---|---|---|
| `0x0` | `ok` | healthy; every requested, owned hook installed |
| `0x1` | `unsupported_kver` | no offset table for the running kernel — refused, no hooks |
| `0x2` | `conflicting_backend` | the *other* kernel backend (`.ko`↔KPM) is loaded — refused (§1.2) |
| `0x3` | `symbol_resolution_failed` | a required kallsyms symbol was missing — refused |
| `0x4` | `partial_hooks` | installed, but some owned hooks did not resolve (see the `hooks` mask) |

`error` is a single dominant code, not a bitset: a refusal is one reason, and
"healthy-but-partial" is the only non-fatal non-zero. The `hooks` mask carries
the per-hook detail when `error = partial_hooks`.

---

## 6. Profiles

One grammar, N profiles. A **profile** is the subset of records a channel
carries, by hook ownership — not by "how many features we shipped". Because
per-app hooks and stats come to **all** backends, every channel carries the full
grammar; there is no "bare" profile.

The parser is **profile-agnostic**: the §4.5 "unknown keyword → skip" rule means
one parser reads any channel and acts only on records it understands. The app is
the only writer of all profiles; each backend reads its own.

| Channel | config records it acts on | emits stats? | emits status? |
|---|---|---|---|
| `.ko` / KPM | `debug`, `target` (kernel-owned mask bits) | yes | yes (§4.3) |
| Zygisk | `debug`, `target` (zygisk-owned mask bits) | yes (§7, if added) | yes, if a read channel is added |
| LSPosed | `debug`, `target` (lsposed-owned mask bits), package/observer records | yes | yes |

A backend ignores `target` mask bits it does not own (`mask & own_hooks`), so the
same `target 0x27fa 0x3ff` line is valid on every channel and each backend takes
its slice.

**Active vs idle (§1.5).** The grammar is the same on every channel, but the app
only writes a *real* config to the LSPosed channel and the **one active** native
channel; the other native channels get an empty config (or none), so a
loaded-but-not-selected native backend parses nothing to act on. Selection is the
app's policy from `status`, not part of the wire format — a backend never knows
whether it is "the active one"; it just applies whatever config it is given (and
the empty config makes it idle).

---

## 7. KPM supercall binding

KPM has no file and no node. The channel is the KernelPatch supercall, already
wired as `KPM_CTL0(vpnhide_kpm_ctl0)` with signature:

```c
static long vpnhide_kpm_ctl0(const char *args, char *__user out_msg, int outlen);
```

The config parser is *already* shared with `.ko` (`apply_targets` →
`vpnhide_parse_target_uids` from `shared/`). KPM reuses the §4 format unchanged;
only the transport differs and `out_msg`/`outlen` (currently `(void)`) become the
read-back channel for `stats` and `status`.

**Confirmed on-device (OPEN-5):** the KPatch-Next `kpatch` CLI **does forward
`out_msg` to stdout** — a probe build that wrote a marker into `out_msg` and
returned its length printed the marker via `kpatch kpm ctl0 vpnhide stats`
(Pixel 8 Pro, KSU-Next, runtime `d05`). The `long` return is **not** printed by
the CLI (a target-arg call that left `out_msg` untouched produced no stdout).
So: **data flows out only through `out_msg`/`copy_to_user`; the return value is
reserved for short codes** (bytes written, or a negative kind/error code), never
for payload.

### 7.1 Dispatch

`ctl0` dispatches on the header `kind`:

```c
static long vpnhide_kpm_ctl0(const char *args, char *__user out_msg, int outlen)
{
    if (kind_is_config(args)) {          /* "vpnhide 1 config\n..." */
        apply_snapshot(args);            /* same shared parser as .ko */
        return nr_target_uids;           /* short code only (NOT surfaced as text) */
    }
    if (kind_is_stats(args) || kind_is_status(args)) {
        char buf[OUT_MAX];               /* "vpnhide 1 stats" / "...status" */
        int n = kind_is_stats(args) ? format_stats(buf, sizeof buf)
                                    : format_status(buf, sizeof buf);
        if (n > outlen)
            n = clamp_to_line(buf, outlen);        /* never mid-record */
        if (_copy_to_user && out_msg)
            _copy_to_user(out_msg, buf, n);        /* the ONLY path data exits */
        return n;
    }
    return -1;
}
```

### 7.2 The one KPM-specific rule: no pagination

`out_msg`/`outlen` is a single fixed buffer per call — unlike seq_file, which
paginates. Therefore:

1. The reader passes a generously large `outlen` (stats for tens of uids is a few
   KiB).
2. The module truncates **only on a line boundary** (`clamp_to_line`), never
   mid-record — otherwise the app parses a garbage tail.
3. Truncation is signalled by the **absence of a trailing `\n`**. The app, seeing
   no trailing `\n`, knows the snapshot is incomplete and either re-reads with a
   bigger buffer or accepts the partial. (This is exactly the §4.1 rule that a
   trailing `\n` is optional on read but meaningful here.)

On write, the whole snapshot must fit in `args`; bound it by `MAX_TARGET_UIDS`
and have the parser truncate honestly at that ceiling (already the case for
`vpnhide_parse_target_uids(..., MAX_TARGET_UIDS)`).

### 7.3 Delivery

Via the KernelPatch KPM ctl0 supercall. KPatch-Next exposes this through its
keyless runtime `kpatch` CLI subcommand **`kpm ctl0`**:

```
kpatch kpm ctl0 vpnhide "<payload>"
```

Write (multi-line payload as one argv argument):

```
kpatch kpm ctl0 vpnhide "vpnhide 1 config
debug 0
target 0x27fa 0x3ff
target 0x2947 0x004"
```

Read (response on stdout — `out_msg`, §7.1):

```
kpatch kpm ctl0 vpnhide "vpnhide 1 stats"
kpatch kpm ctl0 vpnhide "vpnhide 1 status"
```

**The userspace entry must match the running KernelPatch runtime.** The `.kpm`
module itself is cross-version (it loaded and ran on both `c02` and `d05`), but
the userspace ABI is **not** portable. The low 16-bit KPM command ids are the
same (`load=0x1020`, `ctl0=0x1022`, `list=0x1031`), and both runtimes currently
dispatch on those low bits, but the calling conventions differ:

- **APatch:** syscall number `45`, arg0 is the real SuperKey, command word uses
  APatch's `0x1158` marker. APatch's public `apd` CLI manages APM ZIP modules
  only; it does **not** expose `kpm load/ctl0`. APatch's own app controls KPMs
  through native JNI `sc_kpm_*` calls, so the vpnhide activator does the same
  directly. The activator probes `SUPERCALL_HELLO` before load/control, so a
  missing KernelPatch runtime or stale SuperKey fails before a stray original
  syscall can run.
- **KPatch-Next:** syscall number `45`, arg0 is `NULL`, command word uses the
  `0x2026` marker, and the kernel side gates calls by root UID rather than a
  SuperKey. Use the runtime's own `kpatch kpm ...` CLI. With the standalone
  KPatch-Next-Module that binary lives under
  `/data/adb/modules/KPatch-Next/bin/kpatch`.

Do not ship one generic `kpatch` binary for both runtimes.

Caveats from CLI/shell delivery on KPatch-Next: shell-quote the payload (single
quotes; the format contains no `'`). APatch does not pass the key through an
external CLI argv; the activator calls the supercall from its own process.

### 7.4 Distribution & boot integration

The `.kpm` ships as a **thin flashable module** (Magisk/KSU format; APatch reads
it too). The module delivers the binary and a boot script; the actual KPM-load
mechanism is delegated to the activator and runtime. KPatch-Next is keyless;
APatch can also activate at boot when the user has opted into persisting the
SuperKey under `/data/adb/vpnhide/superkey` (root-only DE storage):

- **KPatch-Next (Magisk / KSU), `d05`, keyless — the recommended path.** The boot
  script does everything itself: detect the runtime, enforce single-active (skip
  KPM if the `.ko` is loaded, §1.5), `kpatch kpm load vpnhide.kpm`, then apply the
  persisted config snapshot via `kpatch kpm ctl0`. Fully automatic, no user
  interaction. This is what we recommend users run.
- **APatch, `c02`, SuperKey-required.** If the user enabled
  `rememberSuperkey`, the app persists the key root-only at
  `/data/adb/vpnhide/superkey`, and the boot activator loads/configures APatch
  via direct supercalls after the `SUPERCALL_HELLO` probe. Without that saved
  key, boot writes `awaiting_superkey`; activation resumes after the app supplies
  the key.

This keeps KPatch-Next fully automatic, while APatch is automatic only when the
user explicitly stores the root-only SuperKey for boot.

---

## 8. Drift prevention (C ↔ Rust ↔ Kotlin parity)

Program equivalence is undecidable; "never diverge" is unachievable as a proof.
The achievable goal: make drift **loud** — caught in CI before merge with
probability empirically indistinguishable from never. Layered, each layer
catching what the previous misses.

- **Layer 0 — this spec.** "Compatible" is undefined without an arbiter. Every
  ambiguous decision in §4 (0x policy, hex case, overflow, duplicate uid,
  unknown id/keyword, truncation, trailing `\n`) is a pin point and MUST have a
  vector.
- **Layer 1 — shared golden vectors.** One language-independent file of
  `(input → expected parse)` and `(struct → expected string)`, both directions.
  The C host test and the Rust/Kotlin tests run the *same* vectors against the
  spec. Not codegen'd — the parser is not generated, so a runtime-read data file
  (TSV / `|`-delimited, ten-line C harness) is enough; there is no generated code
  to itself drift. Catches mutual bugs on covered cases; misses uncovered
  corners.
- **Layer 2 — differential testing (the key layer).** Compare the two
  implementations against *each other* over a large generated input space. The C
  parser is already host-compilable (`__VPNHIDE_HOST_TEST`); link it via
  `cc` + `bindgen` into the Rust test binary (**test only** — production stays
  pure Rust, the freestanding constraint is untouched) and assert
  `c_parse(s) == rust_parse(s)` and `c_serialize(x) == rust_serialize(x)` over
  `proptest` inputs. Catches divergence on the long tail; misses *mutual* bugs
  (both wrong identically) — which is why Layer 1 anchors correctness to the spec.
- **Layer 3 — fuzzing the diff oracle.** `cargo-fuzz` harness: raw bytes → both
  parsers → assert they agree (both reject, or both accept with identical
  structure). libFuzzer links freestanding C trivially. Input is trusted
  (root-only channel), so this is not about hostile input — it is about "never
  silently misparse" under pressure: NUL mid-string, overlong lines, leading
  zeros, u32-overflow boundary, mid-line truncation, non-ASCII, duplicate uid.
  The showcase case is KPM truncation: property
  `parse(truncate(serialize(x), n))` is always a valid prefix-subset.
- **Layer 4 — version fuse.** `version` in the header turns any drift that slips
  all tests into a loud refusal rather than a silent misparse (§3).

CI gates all of: C host vectors, Rust vectors + proptest-diff, fuzz smoke (short
on PR, long on schedule), and the registry drift-check (TOML regenerated and
committed). The spec is always the correctness oracle; the diff says "they
diverged", the spec says "who is wrong".

If the format ever grows real structure, the parity cost of two implementations
rises faster than the cost of a single C implementation linked into the hub via
`cc`+`bindgen` (one source, no second parser) — revisit then. For the current
flat text, two impls + this harness is the cheaper, catability-preserving choice.

---

## 9. Rejected alternatives (and why)

Recorded so they are not re-litigated.

- **JSON / protobuf / flatbuffers at the kernel boundary** — their runtimes do
  not compile in freestanding KPM. (JSON-in-userspace-C, as a fork does via a
  daemon, is internally consistent but adds a daemon; we have no hub.)
- **netlink / genetlink** — solves push; we are pull-only, and it is single-
  backend (`.ko`, no KPM equivalent), so it does not unify anything. Reserved for
  a future low-rate kernel→userspace *event* channel if one is ever needed.
- **Binary fixed-layout structs at the kernel boundary (ioctl)** — type-safe and
  a clean single source via `bindgen`, but loses catability (no `cat`/`echo` for
  the agent debugger), and `MAX_*` arrays bake ceilings into the ABI: a new field
  shifts layout and forces a synchronous rebuild across 7 KMIs. Fragile to
  evolution. (If binary is ever needed, prefer TLV over flat structs.)
- **Per-op commands / deltas** (`set`/`add`/`del`/`clear`) — friendlier to
  `echo >>`, but reintroduces stateful, history-dependent application: an
  interrupted batch leaves a half-applied state not derivable from `cat`. Snapshot
  is a pure function of the last write. Rejected.
- **A resident root hub / daemon** — reconsidered explicitly (and after reviewing
  `vpnhide_next`, whose daemon is a narrow netlink poller justified by an IP-spoof
  feature we do not have; its real "hub" is the kmod's `/dev` misc device, which
  couples everything to the `.ko` being loaded — incompatible with "any one of N
  native backends"). Re-rejected: a resident named root process is enumerable
  (`ps`/`/proc`), exactly the fingerprint this project minimises; it adds
  keep-alive/SELinux/IPC surface; and it does not remove the irreducible
  in-`system_server` Kotlin reader. Its one real win — *live* reaction to events
  without the app open — is not a requirement (targets change rarely; boot +
  app-open re-resolution suffice), and its other wins (single code path, root
  syscalls, coordinated single-active) are had by the app-as-orchestrator + a
  thin boot script + the kernel guard (§1.5) with no resident process. Revisit
  only if a feature needs a live event loop. The narrow KPM coordination it would
  buy is already covered by the kernel guard.
- **Unified single state file across backends** — multi-writer across three
  SELinux domains, torn reads, one label for all readers. State stays
  per-backend.
- **Routing LSPosed/Java state through the kmod via ioctl** (a fork's approach) —
  centralises state but couples LSPosed to the kmod being loaded and needs
  `system_server` sepolicy for the device node. We keep backends autonomous.

---

## 10. Open decisions (resolve before freezing version 1)

- **OPEN-1 — `0x` prefix. RESOLVED: mandatory on data fields.** It is a
  visual/parse anchor at ~zero cost, and "mandatory" (not "optional") removes the
  two-spellings-of-one-number drift seam.
- **OPEN-2 — agent self-description level. RESOLVED: positional-after-keyword.**
  `target <uid> <hookmask>` — the keyword names the record, fields are positional.
  `key=value` buys marginal readability for extra bytes and a second split; the
  keyword already self-describes for an agent reading raw.
- **OPEN-3 — stats counter type. RESOLVED: `u64` cumulative-since-load.** Reads
  are non-destructive (no reset-on-read race between two readers), it never
  wraps in practice, and deltas are the app's job — which suits the pull model
  (§2). `count` is therefore a `u64` hex value; kernel counters are per-CPU `u64`.
- **OPEN-4 — `.ko` stats node. RESOLVED: one folded node, renamed
  `/proc/vpnhide_ctl`.** `.proc_write` takes the config snapshot, `.proc_read`
  returns status+stats — one node, no new ones. Renamed from `vpnhide_targets`
  for **semantic accuracy** (it is now a control channel, not just targets), NOT
  for stealth: the node is `0600` root-only and the threat model is an
  *unprivileged* app (root-level detection is explicitly out of scope — you cannot
  defend a device whose owner gives root to a VPN-detector), so the name is never
  observable by an adversary. The persistent `/data` files keep their names.
  Debug folds in here too (OPEN-6), so `/proc/vpnhide_debug` goes away. (KPM has
  no node either way — supercall read.)
- **OPEN-5 — KPM userspace transport. RESOLVED (on-device).** KPatch-Next
  exposes `kpm ctl0 <name> <payload>` through its keyless `kpatch` CLI, and the
  CLI **does forward `out_msg` to stdout** — so no extra root binary is needed
  for read-back there. APatch does not expose `kpm load/ctl0` through `apd`; the
  activator calls the runtime-specific supercalls directly with the saved
  SuperKey. The `.kpm` is cross-version, but the userspace transport is not.
- **OPEN-6 — `debug` placement. RESOLVED: folded into the config snapshot.**
  `debug <flag>` is the one global (non-`target`) config record (§4.3). Today
  each backend has its own `debug_logging` file (and the `.ko` a
  `/proc/vpnhide_debug` node); folding it into the config snapshot removes those
  per-backend files/nodes (less enumerable surface), makes it atomic with the
  rest of config, and needs one parse instead of a second channel.
- **OPEN-7 — self-documenting read. RESOLVED: yes, emit an in-band header.** The
  read side prepends a one-line `#` comment (`# vpnhide v1 — WRITE REPLACES ENTIRE
  STATE …`) before the current state. It is a comment line (ignored by parsers,
  §4.1), so it costs nothing structurally and lets an agent learn the grammar +
  the replace-whole semantics from a single `cat` (§2). All seven OPEN items are
  now resolved; `version 1` is frozen.
