# App mutation transport

Status: implemented transport, packaged with the APK; not connected to the runtime
config coordinator yet. Existing app writes still use `CanonicalConfigRepository`
and `suExec`. This is the next foundation after the
[pure transition cores](app-state-transitions.md#13-first-implementation-pure-transition-cores).

## Why a separate executable

Destroying the Java `Process` for `su` does not establish that a privileged child
stopped. A delayed root authorization can also launch a command after the caller
already timed out. Both cases need evidence beyond the app's job/PID lifetime.

`vhmutate` is a short-lived supervisor, built in `lsposed/native/` from the existing
Rust dependencies. It holds a nonblocking exclusive `flock` and enables Linux
[`PR_SET_CHILD_SUBREAPER`](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html).
Orphaned descendants return to this supervisor; it uses
[`waitpid`](https://man7.org/linux/man-pages/man2/wait.2.html) until its shell has an
exit status and there are no remaining children. An orphan that fails makes the
receipt unsuccessful even if the original shell returned zero.

This runs in its own executable, never in JNI or `system_server`: it must not
reap unrelated app children. A persistent daemon is not needed. The lock alone is
insufficient: if the supervisor dies, the lock is released while a child can live.

## Lifetime metadata

Production staging uses `/data/adb/vpnhide/app-state/` (root-owned, `0700`).
`lane/lock` is a permanent inode; `lane/state.json` is the one latest receipt,
atomically replaced through `lane/state.next` with file and directory `fsync`.
Metadata files are `0600`; no broad SELinux policy is installed.

The receipt contains protocol version 1, a monotonic revision, boot ID, random
app-session ID, command sequence, status, shell exit code and an orphan-failure
flag. It contains no command, config, secret or shell output. It is lifetime
metadata, not a persistent operation queue; there is no replay log.

Statuses:

| Status | What is known |
|---|---|
| `idle` | Session has not dispatched a command |
| `running` | Dispatch may have happened; terminal evidence is absent |
| `finished` | Shell and all descendants terminated; exit results are recorded |
| `not_started` | This sequence is consumed without executing its command |

`running` is durable **before** spawn. `finished` is written only after draining
all descendants. A same-boot `running` receipt blocks mutation even if the lock
is available and the supervisor PID is gone. There is deliberately no PID-based
reset. A verified new boot proves previous-boot processes stopped, but does not
reconstruct their outcomes or resume the old operation.

An existing lane with missing, malformed or unsupported metadata fails closed;
it is never silently initialized as empty. Do not delete/replace the lock or lane
while an old invocation could exist. Explicit repair and the initial adoption
boundary are coordinator/migration work.

## Protocol and late-launch fencing

Invocation: `vhmutate STATE_DIRECTORY CANONICAL_PATH VERB ...`. Paths are explicit
to support isolated host/device tests. Android execution requires UID 0. All
commands return one JSON reply (`version`, `status`, `boot`, `state`,
`config_status`, `config`). `busy` and `unavailable` never establish completion.

| Verb | Arguments | Behavior under the lock |
|---|---|---|
| `inspect` | none | Read the latest receipt and, if quiescent, the canonical file |
| `open` | current boot, expected revision, new session UUID | Compare-and-set session replacement after predecessor quiescence; an identical retry returns the current receipt without resetting it |
| `run` | current boot, session UUID, next sequence | Validate identity/order, persist `running`, execute exactly once, drain, persist result |
| `recover` | current boot, session UUID, attempted sequence | Read existing receipt, or consume an undispatched sequence as `not_started` so its late launch cannot run |

A sequence must be exactly the successor of the last consumed sequence. Duplicate
`run` is rejected. An old session's command or delayed `open` cannot replace the
new session. An obsolete boot ID cannot dispatch in the current boot.

`recover` does not repeat a config/secret/activation command. It may update
**transport metadata** to fence a not-yet-dispatched launch. This is necessary:
a strictly read-only check cannot prevent a command still waiting in `su` from
starting immediately after that check. The transition contract's “read-only
recovery” means no replay or new application configuration effects, not a ban on
this metadata fence. The coordinator still owns the initial recovery plus one
repeat, then Paused; the transport never replenishes or runs that retry policy.

Command input is `vpnhide-script 1 BYTE_LENGTH\n` followed by exactly that many
UTF-8 bytes and EOF. Full framing is validated before taking the lane lock. A
truncated but syntactically valid script prefix is not executable input. The
supervisor feeds the validated script to the shell through an anonymous `memfd`,
not argv or an on-disk script. Shell stdout/stderr are discarded; operational
errors must be presented through typed phase results. `suExec` command-preview
logging is not used by this transport.

## Bounds and interpretation

- `RootProcessRunner` waits at most 10 seconds by default, including writing stdin
  and draining output. Blocking process creation and inherited pipes cannot extend
  the caller's wait. Three outstanding executions are allowed per runner; hung
  pipe owners retain their slots. The coordinator must retain one runner across
  staging/initialization/retries instead of creating a new runner per attempt.
- The native supervisor allows 10 seconds to receive the complete input and 120
  seconds to drain the command's descendants. On drain expiration it leaves
  `running` durable. It does not claim cancellation, kill recycled PIDs, or permit
  further mutations. Lost supervisor evidence can therefore require a reboot.
- Scripts and canonical reads are capped at 2 MiB each, metadata at 4 KiB, JVM
  response retention at 4 MiB. Oversized/truncated data is unavailable, never a
  default empty config. Canonical readback rejects symlinks and nonregular files,
  so a FIFO cannot block the lane.
- Canonical readback distinguishes missing, invalid and unavailable. Parsing uses
  the existing `parseCanonicalConfig`; there is no second canonical parser.
- For persistence, readback must match the candidate or base before interpreting
  it as confirmed or known failure. A nonzero shell exit alone does not imply that
  replacement did not happen. A third value or unreadable canonical stays Unknown.
- For activation, completion describes the invocation, not consumption by every
  target process. Backend readiness, partial activation repair and private secret
  readback remain responsibilities of the typed effect adapters.

The executable is staged under a content-derived filename. Existing executables
are not overwritten, and their SHA-256 is verified before use. Staging can finish
late without altering lane metadata or gaining permission to dispatch an effect.

## Validation and remaining integration

Host tests run the real executable and cover session replacement, duplicate and
delayed dispatch, truncated input, recovery fencing, descendant lifetime, killed
supervisors, partial writes, redaction, corrupt/missing metadata, old boot records,
and FIFO readback. Kotlin tests cover bounded process I/O, receipt validation,
phase interpretation and immutable executable staging.

Validation on 2026-09-15: 581 Kotlin unit tests (11 new transport tests) and all
24 native Rust tests (11 new transport tests) passed. Kotlin ktlint, detekt, CPD
and Android lint passed; Rust clippy passed on the host and both ARM Android
targets. The helper was built for `arm64-v8a` and `armeabi-v7a`; the device checks
below exercise the ARM64 artifact.

Device reproduction (isolated scratch files, no APK install or live activators):

```sh
uv run scripts/test-root-transport.py --serial SERIAL \
  --binary lsposed/app/build/rustNative/assets/bin/arm64-v8a/vhmutate
```

Verified on 2026-09-15 on Pixel 8 Pro, through `adb shell su` in the KernelSU
`u:r:ksu:s0` domain with SELinux enforcing. The tests exercised late-launch
rejection, fencing, a partial write, a detached descendant, and supervisor death.
They removed their scratch files after the controlled descendants finished.
Other root managers and app-originated `su` permission domains are not validated
by this test. A previous-boot receipt is simulated on the host; no device reboot
was performed.

Before runtime adoption:

1. Establish a boundary for commands launched by the **old** untracked transport.
   A newly created lane does not prove legacy `su` descendants or delayed root
   prompts are absent; first adoption needs a verified boot boundary or equivalent
   evidence. Likewise, external boot services/manual root writers are not covered
   by this app-only lock.
2. Connect typed config intents and process-owned initialization/dispatch to the
   existing repository; retain per-phase receipts until their operation settles.
   A fresh session may not turn unknown old outcomes into successful operations.
3. Move every app writer, including startup/capture/bridge paths, through that
   coordinator before enabling the new lane. Add UI progress, recovery/repair
   actions, draft ownership and structured bridge results there.
   Preserve the existing `native_target_cap` warning through typed activation
   evidence: this transport currently discards raw command output, so redirecting
   the old activator calls unchanged would lose that warning.
4. Validate real canonical writes, activation/secret failures, Activity/process
   recreation and app-originated root execution on supported devices/managers.

The running app's toggle behavior is unchanged at this transport-only stage.
