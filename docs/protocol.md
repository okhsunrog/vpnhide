# vpnhide — control & stats protocol

Single source of truth for the runtime wire exchanged with the native backends
(`.ko`, KPM, Zygisk) and for the telemetry wire emitted by `.ko`, KPM, and
LSPosed. This document is the **arbiter** for those bytes: when two
implementations disagree, this file decides who is wrong. Every implementation
(C / Rust / Kotlin) and every test vector references it.

Status: **two protocols** over one lexical core and one hook-id registry —
`control` **v2** (the `config` payload) and `telemetry` **v1** (`stats` +
`status`). Both frozen; all seven OPEN items (§10) are resolved. See §3 for why
the split exists and what it costs to move either number.
Key shape: three `kind`s — `config` (in), `stats` + `status` (out, §4.3); UID
is the key on every control channel including Zygisk; `u64` cumulative counters;
`debug` folded into config; the `.ko` channel is one folded node
`/proc/vpnhide_ctl` (write=config, read=status+stats); KPM uses the `kpm ctl0`
supercall (§7).
Threat model: an **unprivileged** app — root-level detection is out of scope.

Configuration storage, activation policy, optional features, and loader
arguments are deliberately outside this protocol. Their authoritative documents
are [storage.md](storage.md) and [state.md](state.md); backend loader ABIs are in
[the `.ko` README](../kmod/README.md) and
[the KPM README](../kmod/kpm/README.md).

---

## 1. Scope and runtime bindings

### 1.1 Contract boundary

This document defines four things:

- the control-v2 and telemetry-v1 grammars;
- the shared hook-id and error-code registries;
- the backend profiles that select which records and hook bits each consumer
  understands;
- transport framing where it affects the bytes, notably KPM truncation and
  stats pagination.

It does **not** define the canonical JSON schema, package-to-UID projection,
native-backend selection, boot order, APatch credential persistence, or the
arguments used to load `.ko`/KPM artifacts. In particular,
`settings.optionalFeatures` and the `filesystem_hiding=1` loader argument are
not control-v2 records. See [storage.md](storage.md) for desired state and
activation, and [state.md](state.md) for paths, lifetimes, and boot sequencing.

### 1.2 Runtime channels

The grammar is shared, but the transports and supported directions differ:

| Runtime | Control v2 in | Telemetry v1 out | Consumer/producer |
|---|---|---|---|
| `.ko` | write `/proc/vpnhide_ctl` | read `/proc/vpnhide_ctl` (`seq_file`) | kernel |
| built-in (`CONFIG_VPNHIDE=y`) | write `/proc/vpnhide_ctl`, byte-for-byte the `.ko` channel | read `/proc/vpnhide_ctl` (`seq_file`) | kernel |
| KPM | `ctl0` `args` | `ctl0` `out_msg` | kernel |
| Zygisk | module-dir `targets.txt` | none | forked app process |
| LSPosed | none; reads canonical JSON directly | `/data/system/vpnhide_lsposed_state` | `system_server` |

The Zygisk module reads `targets.txt` through the privileged module-directory fd
provided before app specialisation (`openat(dir_fd, "targets.txt", …)`). It
reloads on the next process launch. KPM is the only request/response transport
without a file or proc node; its binding is specified in §7. LSPosed participates
only in telemetry v1: its configuration input belongs to the storage contract.

### 1.3 Implementation ownership

| Language | Component | Wire responsibility |
|---|---|---|
| C | `.ko` + KPM | parse control config; emit telemetry. Freestanding core in `kmod/shared/vpnhide_logic.h`. |
| Rust | `crates/protocol` + activators | format control snapshots, validate native telemetry, and implement KPM framing. |
| Rust | Zygisk cdylib | parse its control profile from `targets.txt`. |
| Kotlin | app | parse telemetry for dashboard, diagnostics, and statistics. |
| Kotlin | LSPosed hook | emit telemetry-shaped status/stats from `system_server`; config comes from canonical JSON. |

Parser parity across C, Rust, and Kotlin is held by golden vectors and
differential testing (§8), not by shared runtime code.

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
- **Debugging uses raw `adb` reads and writes** (`cat`/`echo`). The format must
  survive shell quoting (no `"`, `$`, backtick, `\` in payload) and be
  self-describing enough to read without external context. This is why keywords
  are words, not magic numbers.
- **Pull only.** Kernel aggregates per-(uid, hook) counters; userspace reads
  periodically. No push, no per-hit events on the hot path.

---

## 3. Semantics

- **Snapshot, not deltas.** One write = the entire desired state. The reader
  replaces its state wholesale. There is no add/remove/clear. This makes state a
  pure function of the last write — it never depends on history, which removes
  the whole class of "partially-applied" / "stale leftover" bugs. It also matches
  `echo > node` (truncating redirect) and atomic replacement of Zygisk's derived
  `targets.txt` snapshot.
- **Mandatory header gating.** A payload without a valid header (§4.2) is
  rejected *whole* — a stray `echo 'debug 0' > node` does not silently wipe
  state, it errors. "Valid full state, or a loud refusal, never silent-partial."
- **Version gates compatibility.** A reader knows only its own version. A payload
  whose version is not exactly the current version for its kind is rejected
  whole (not parsed). This turns
  any drift that slips past tests into a loud refusal instead of a silent
  misparse — exactly what is wanted across the 7-KMI matrix where
  userspace↔kernel skew is inevitable.
- **Two protocols, versioned apart, because their readers ship apart.** The
  version boundary follows the *delivery* boundary, not the shape of the data:

  | protocol | kinds | version | writer → reader | cost of a bump |
  |---|---|---|---|---|
  | **control** | `config` | **2** | activator → backend | none — both are files in one flashable zip and update atomically |
  | **telemetry** | `stats`, `status` | **1** | backend → app | breaks an older APK's dashboard and diagnostics; the app updates on its own schedule |

  `stats` and `status` are one protocol, not two: a single `/proc/vpnhide_ctl`
  read returns both back to back, so they have one reader and one delivery, and
  versioning them apart could not express anything.

  What the two share is a *lexical core* (§4.1), the header shape (§4.2), and
  the hook-id registry (§5) — the same id space indexes a `config` hookmask bit
  and a `stats` `hook_id`. Shared vocabulary, separate wires.

  **Do not move `telemetry` without shipping the app in step.** `control` may be
  bumped freely; there is no compatibility window to preserve, and none is kept
  — a v1 `config` parser no longer exists.

---

## 4. Wire format

### 4.1 Lexical rules

- **Encoding:** ASCII only, bytes `0x20`–`0x7E` plus `\n`. Any other byte in a
  line → reject that line.
- **Line separator:** `\n` only. On read, a trailing `\r` (CRLF) is stripped. On
  write, never emit `\r`. A trailing `\n` at end of payload is optional on read
  (but see §7.2 for its meaning under KPM truncation).
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
  §4.4 does not apply to it). Current: `2` for `config`, `1` for `stats` and
  `status` (§3). The fuse is applied **per protocol**, so the same number can
  pass on one kind and fail on another: `vpnhide 2 config` is valid and
  `vpnhide 2 stats` is not.
- `<kind>` — `config` (app → backend), `stats` (backend → app), or `status`
  (backend → app: module health + errors, §4.3).

A payload without this header, or with a version other than the reader's exact
current version for that kind,
is rejected whole (§3).

"Significant" is decided by **non-blank, non-comment alone** — a non-ASCII line
is significant. So if the first significant line is non-ASCII, it *is* the line
that MUST be the header; being an invalid header, it rejects the payload whole.
The §4.1 "reject that line" (skip-and-continue) rule applies to **records**,
once a valid header is established — it does **not** let the header search step
over a non-ASCII first significant line to find a header on a later line.

### 4.3 Records

**config** (`kind = config`, control v2):

```
debug <flag>
default <hookmask>
targets <hookmask> <uid> [<uid> ...]
end <count>
```

- `debug <flag>` — `flag` is the literal `0` or `1`. At most one `debug` line;
  absent ⇒ unchanged-from-default (define default as `0`).
- `default <hookmask>` — the hookmask applied to every uid **not** listed in a
  `targets` record. Absent ⇒ `0`, which makes the listed set the apps to act on:
  the blacklist the project ships. A non-zero default inverts the reading —
  everyone is acted on and the listed set becomes the exception list, which is
  the mechanism a whitelist mode rides on. The wire carries the mechanism
  whether or not a producer uses it; emitting a non-zero default is a producer
  decision, and a backend that cannot honour one MUST reject the payload rather
  than read a whitelist config as a blacklist.
- `targets <hookmask> <uid> ...` — one record per distinct hookmask, carrying
  every uid that shares it. `hookmask` is a hex bitset of enabled hooks (bit N ⇔
  `hook_id` N, §5/registry). Repeatable. Grouping is where the density is: the
  keyword and the mask amortise across the whole run, and in practice nearly
  every app carries the same mask. That matters because the KPM's transport caps
  a whole config at 1024 bytes (§7).
  Duplicate `uid` ⇒ last-wins. A **malformed uid inside a record rejects the
  payload whole** — unlike an unknown keyword, it would desync `end`.
  **UID is the key for *every* backend**, including Zygisk: the kernel backends
  have no PackageManager so they cannot key on package names, and Zygisk (which
  runs in the target's own process and could match either way) keys on `getuid()`
  for one grammar across all channels. The package→UID resolution is the
  producer's job (the activator), the same for all backends.
- `end <count>` — **mandatory**. `count` is the total number of uid tokens the
  producer wrote. Missing, or not equal to what the reader counted ⇒ reject the
  payload whole. This is the truncation fuse: the KPM copies a config through a
  fixed 1024-byte buffer and truncates it *silently*, so without `end` a
  too-large config would quietly apply as a partial target set. More uids than
  the backend can store is likewise a reject, never a silent drop.

A parsed target set is **sorted ascending by uid**. That is a contract, not an
implementation detail: it is what lets a backend binary-search the set on every
hooked call instead of walking it, and it makes the parsed form independent of
the order a producer grouped uids in.

**stats** (`kind = stats`):

```
<uid> <hook_id>:<count> <hook_id>:<count> ...
```

- One line per uid that has any non-zero counter. Sparse: only non-zero
  `hook_id:count` pairs are emitted. `hook_id` and `count` are hex; `count` is a
  `u64` **cumulative since the backend loaded** (OPEN-3) — reads never reset it,
  so two readers don't race and the app computes deltas itself.

Numeric asymmetry is deliberate too: control v2 writes **bare** hex, telemetry
v1 keeps the `0x` prefix (§4.4). The prefix costs two bytes on every number, and
only control has a payload ceiling tight enough for that to decide how many apps
fit.

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

- `backend <id>` — which backend answered (registry ids from `data/hooks.toml`:
  `0x0` .ko, `0x1` KPM, `0x2` Zygisk, `0x3` LSPosed, `0x4` built-in).
- `kver <kver>` — running kernel version as the backend sees it (KernelPatch
  `kver`, e.g. `0x6019d`); `0x0` for non-kernel backends.
- `hooks <installed_mask>` — bitset of hooks that *actually* installed, in the
  same id space as a config `hookmask`. Lets the app show "requested vs active".
- `error <code>` — `0x0` = healthy; non-zero is the single dominant fault code
  (§5.1). At most one `error` line.

`status` exists because a kernel backend can refuse or only partially install for
reasons the app cannot otherwise see (no node, no logs unless `debug`). It turns
those into a readable state instead of a silent no-op. Activation diagnostics use
the same error vocabulary; in particular, a KPM loader can report
`conflicting_backend` when it detects the `.ko`. The guard itself is an activation
rule documented in [storage.md](storage.md#43-native-backend-selection-and-safety).

### 4.4 Numeric primitive

Hex for every data field (`uid`, `hookmask`, `hook_id`, `count`) — with the
prefix decided **per protocol**:

```
control v2 (config):            one or more hex digits, NO prefix
telemetry v1 (stats, status):   0x  followed by one or more hex digits
```

- Within a protocol the spelling is **exactly one** of the two. The prefix is
  mandatory in telemetry and forbidden in control; it is never *optional*,
  because an optional prefix is two valid spellings of one number and therefore
  a drift seam (OPEN-1). A `0x`-prefixed uid inside a control `targets` record
  is malformed, and since a malformed uid desyncs `end`, it rejects the payload.
- Control drops the prefix because it is the only protocol with a payload
  ceiling tight enough to care: two bytes per number, against the KPM's
  1024-byte config transport, is a direct tax on how many apps fit (§7).
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

config (activator → backend):

```
vpnhide 2 config
debug 0
targets 3ff 27fa
targets 4 2947
end 2
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

Current shared kernel hooks (`.ko` / KPM, 11): `fib_route_seq_show`,
`ipv6_route_seq_show`, `rtnl_fill_ifinfo`, `inet_fill_ifaddr`,
`inet6_fill_ifaddr`, `dev_ioctl`, `sock_ioctl`, `fib_dump_info`, `rt6_fill_node`,
`fib_nl_fill_rule`, `socket_bind_interface`. Both kernel backends and Zygisk
also own the optional `filesystem_iface_paths` capability bit. For `.ko`/KPM,
whether the global hook group is installed is outside control v2. For Zygisk,
the activator includes the bit in a target mask only while the same canonical
feature is enabled; the process-local libc group reports successful
installation through the app heartbeat rather than telemetry v1. See
[storage.md](storage.md#22-optional-features). Current LSPosed Java hooks (8):
`lsposed_link_properties`,
`lsposed_network_capabilities`, `lsposed_network_info`, `lsposed_network`,
`lsposed_connectivity_result`, `lsposed_connectivity_callback`,
`lsposed_connectivity_network`, `lsposed_package_visibility`. Current Zygisk
libc hooks (9): `zygisk_ioctl`, `zygisk_getifaddrs`, `zygisk_openat`,
`zygisk_recvmsg`, `zygisk_recv`, `zygisk_recvfrom`,
`zygisk_recvfrom_chk`, `zygisk_setsockopt`, `filesystem_iface_paths`.

### 5.1 Error codes (`status`)

A small fixed enum, codegen'd from the same registry TOML as the hooks, so the
numbers are one source of truth across C / Rust / Kotlin (a status reader must
not invent its own codes). The `error` field of a `status` payload (§4.3) is one
of:

| code | name | meaning |
|---|---|---|
| `0x0` | `ok` | healthy; every requested, owned hook installed |
| `0x1` | `unsupported_kver` | no offset table for the running kernel — refused, no hooks |
| `0x2` | `conflicting_backend` | KPM activation found the `.ko` installed or live and refused before loading/configuring KPM |
| `0x3` | `symbol_resolution_failed` | a required kallsyms symbol was missing — refused |
| `0x4` | `partial_hooks` | installed, but some owned hooks did not resolve (see the `hooks` mask) |

`error` is a single dominant code, not a bitset: a refusal is one reason, and
"healthy-but-partial" is the only non-fatal non-zero. The `hooks` mask carries
the per-hook detail when `error = partial_hooks`.

---

## 6. Profiles

One grammar, N profiles. A **profile** is the subset of records and hook bits a
consumer or producer supports. It is not a storage schema and does not imply
that every runtime implements both protocol directions.

The parser is **profile-agnostic**: the §4.5 "unknown keyword → skip" rule means
one parser reads any channel and acts only on records it understands. Each
backend reads its own profile.

| Channel | config records it acts on | emits stats? | emits status? |
|---|---|---|---|
| `.ko` / built-in / KPM | `debug`, `default`, `targets`, `end` (kernel-owned mask bits) | yes | yes (§4.3) |
| Zygisk | `debug`, `targets`, `end`; its activator rejects a non-zero `default` because it cannot inject into every unlisted process | no | no; its app heartbeat is not telemetry v1 |
| LSPosed | — does **not** consume control v2; it reads canonical JSON directly | yes | yes |

A control consumer ignores mask bits it does not own (`mask & own_hooks`). The
same global-mask payload can therefore be delivered to any native backend; each
backend takes only its slice. Telemetry producers use the same global hook IDs,
so the app can merge `.ko`/KPM and LSPosed counters without remapping them.

Native-backend selection is not encoded in the payload. A backend simply applies
the control snapshot it receives. The app-side priority and the KPM/`.ko` safety
guard are activation policy; see
[storage.md](storage.md#43-native-backend-selection-and-safety).

---

## 7. KPM supercall binding

KPM has no file and no node. The channel is the KernelPatch supercall, already
wired as `KPM_CTL0(vpnhide_kpm_ctl0)` with signature:

```c
static long vpnhide_kpm_ctl0(const char *args, char *__user out_msg, int outlen);
```

The config parser is *already* shared with `.ko` (`apply_targets` →
`vpnhide_parse_config` from `kmod/shared/`). KPM reuses the §4 format unchanged;
only the transport differs, with `out_msg`/`outlen` forming the read-back channel
for `stats` and `status`.

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
    if (kind_is_config(args)) {          /* "vpnhide 2 config\n..." */
        apply_snapshot(args);            /* same shared parser as .ko */
        return nr_target_uids;           /* short code only (NOT surfaced as text) */
    }
    if (kind_is_stats(args) || kind_is_status(args)) {
        char buf[OUT_MAX];               /* one stats page, or all of status */
        int n = kind_is_stats(args) ? format_stats_page(buf, sizeof buf,
                                                        parse_after(args))
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

### 7.2 KPM stats cursor

`out_msg`/`outlen` is a single fixed 4096-byte buffer per call. Stats therefore
uses an additive UID cursor while keeping telemetry v1 and the ordinary §4.3 UID
records. The first request is the legacy request; later pages add one record:

```text
vpnhide 1 stats
after 0x27ff
```

Every new-backend response ends in an integrity trailer:

```text
vpnhide 1 stats
0x2800 0x0:0x12 0x19:0x5
0x2801 0x2:0x7
page 0x27ff 0x2801 0x2
```

The fields are the requested cursor, the next cursor (or literal `done`), and
the number of UID rows carried by this page. UID rows are strictly increasing
and contain only UIDs greater than `after`. The activator validates the echoed
cursor, monotonic next cursor, row count, and every numeric field while holding
the cross-process ctl0 lock across the full loop. It then folds all pages into
one ordinary `vpnhide 1 stats` block, so the APK parser is unchanged.

A non-final page deliberately omits the trailing `\n`; the `page` record itself
is complete. An old activator therefore applies its existing truncation rule and
drops the incomplete stats instead of presenting a partial total. A final page
uses `page <requested> done <count>\n`. Conversely, a new activator accepts a
newline-terminated legacy response with no `page` trailer as a complete
single-page snapshot from an older backend.

The `.ko` transport needs no cursor: `/proc/vpnhide_ctl` uses `seq_operations`
with one UID per record, so the kernel streams arbitrarily many complete rows to
the reader without constructing a fixed-size snapshot.

On write, the whole snapshot must fit in `args`, including its trailing NUL.
Bound it by `MAX_TARGET_UIDS`, check the formatted byte length before the
supercall (distinct per-app masks cost extra group headers), and have the parser
reject honestly at its target ceiling (already the case for
`vpnhide_parse_config(..., MAX_TARGET_UIDS)`).

### 7.3 Delivery

Via the KernelPatch KPM ctl0 supercall. KPatch-Next exposes this through its
keyless runtime `kpatch` CLI subcommand **`kpm ctl0`**:

```
kpatch kpm ctl0 vpnhide "<payload>"
```

Write (multi-line payload as one argv argument):

```
kpatch kpm ctl0 vpnhide "vpnhide 2 config
debug 0
targets 3ff 27fa
targets 4 2947
end 2"
```

Read (response on stdout — `out_msg`, §7.1):

```
kpatch kpm ctl0 vpnhide "vpnhide 1 stats"
kpatch kpm ctl0 vpnhide "vpnhide 1 status"
```

**The userspace entry must match the running KernelPatch runtime.** The `.kpm`
module itself is cross-version, but the userspace ABI is **not** portable. Both
runtimes use low command id `ctl0=0x1022`, but their calling conventions differ:

- **APatch:** syscall number `45`, arg0 is the real SuperKey, command word uses
  APatch's `0x1158` marker. APatch's public `apd` CLI manages APM ZIP modules
  only; it does **not** expose `kpm ctl0`. APatch's own app controls KPMs through
  native JNI `sc_kpm_*` calls, so the vpnhide activator does the same directly.
  The activator probes `SUPERCALL_HELLO` before control, so a
  missing KernelPatch runtime or invalid credential fails before a stray
  original syscall can run. Credential storage is specified in
  [storage.md](storage.md#6-apatch-superkey).
- **KPatch-Next:** syscall number `45`, arg0 is `NULL`, and the kernel side gates
  calls by root UID rather than a SuperKey. The activator issues no raw
  command-word marker here — it drives KPatch-Next through the runtime's own
  `kpatch kpm ...` CLI. With the standalone
  KPatch-Next-Module that binary lives under
  `/data/adb/modules/KPatch-Next/bin/kpatch`.

Do not ship one generic `kpatch` binary for both runtimes.

Caveats from CLI/shell delivery on KPatch-Next: shell-quote the payload (single
quotes; the format contains no `'`). APatch does not pass the key through an
external CLI argv; the activator calls the supercall from its own process.

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

The `status_fields` vectors exercise the Rust/Kotlin status consumers; C emits
status and checks its existing emission vectors. Both consumers preserve missing
fields separately from zero. Display callers may supply defaults, but kernel
ownership checks require a complete status: an absent backend must never become
backend zero (kmod). Backend identities come from the generated registry enum;
shell commands must not parse this wire format independently.

CI currently gates C host vectors, Rust/Kotlin vectors, the Rust↔C proptest
differential oracle, and the registry drift-check (TOML regenerated and
committed). Layer 3 remains the next hardening step; no fuzz-smoke gate is
claimed until a bounded PR corpus and scheduled long run are actually wired.
The spec is always the correctness oracle; the diff says "they diverged", the
spec says "who is wrong".

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
  direct debugging), and `MAX_*` arrays bake ceilings into the ABI: a new field
  shifts layout and forces a synchronous rebuild across 7 KMIs. Fragile to
  evolution. (If binary is ever needed, prefer TLV over flat structs.)
- **Per-op commands / deltas** (`set`/`add`/`del`/`clear`) — friendlier to
  `echo >>`, but reintroduces stateful, history-dependent application: an
  interrupted batch leaves a half-applied state not derivable from `cat`. Snapshot
  is a pure function of the last write. Rejected.
---

## 10. Historical wire decisions

- **OPEN-1 — `0x` prefix. RESOLVED per protocol.** Telemetry v1 keeps it
  mandatory as a visual/parse anchor; control v2 forbids it to reclaim two
  bytes per number under the KPM transport ceiling. Neither protocol makes the
  prefix optional, so each value still has one spelling.
- **OPEN-2 — self-description level. RESOLVED: positional-after-keyword.**
  Control v2 uses `targets <hookmask> <uid>...`: the keyword names the grouped
  record and fields remain positional. `key=value` buys marginal readability
  for extra bytes and a second split; the keyword is already readable in a raw
  capture.
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
  for read-back there. APatch does not expose `kpm ctl0` through `apd`; the
  activator calls the runtime-specific supercall directly. Authentication and
  credential persistence belong to [storage.md](storage.md#6-apatch-superkey).
  The `.kpm` is cross-version, but the userspace transport is not.
- **OPEN-6 — `debug` placement. RESOLVED: folded into the config snapshot.**
  `debug <flag>` is the one global (non-`target`) config record (§4.3). Earlier
  designs used per-backend `debug_logging` files and a `.ko`
  `/proc/vpnhide_debug` node. Folding the flag into the snapshot removed those
  extra channels, made debug atomic with the rest of config, and reduced parsing.
- **OPEN-7 — self-documenting read. RESOLVED: yes, emit an in-band header.** The
  read side prepends a one-line `#` comment (`# vpnhide v1 — WRITE REPLACES ENTIRE
  STATE …`) before the current state. It is a comment line (ignored by parsers,
  §4.1), so it costs nothing structurally and exposes the grammar +
  replace-whole semantics in a single `cat` (§2). All seven OPEN items are
  now resolved; both protocols (control v2, telemetry v1) are frozen.
