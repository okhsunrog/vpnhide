# VPN Hide AI support (sketch)

Server-side assistant between the APK and an LLM. The app sends the installed
version, a compact diagnostic snapshot, and a question. This process owns the
system prompt, the model, and the only tools the model may call.

The crate is a skeleton. It builds and serves the HTTP contract. It does not
call a model, file a GitHub issue, or store a diagnostic archive.

No design note named "AI Support, диагностика и GitHub Issue Gateway" is
checked in under `docs/` or `docs/notes/`. The boundaries below are the ones
this sketch implements. Diagnostics vocabulary follows
[docs/diagnostics.md](../../docs/diagnostics.md) and
[docs/debug-bundle.md](../../docs/debug-bundle.md).

This is not the on-device core daemon. It does not read the journal, own
config, or run an activator.

## Boundaries

- No shell. `search_docs`, `search_code`, `read_file`, and `search_issues` are
  in-process functions. Nothing in this crate spawns a command.
- No git history. Code and docs are read from one checkout of the version in
  the request. There is no `git log`, no `git show`, and no diff against
  another release. A `changelog_between` tool can be added later; it is not
  the default.
- A missing checkout is reported as missing. The server does not fall back to
  a different version's tree.
- The client cannot send a system prompt, a tool list, or a model name.
  Unknown JSON fields are rejected.
- `search_issues` returns `not_configured`. It does not call GitHub.
- `POST /api/v1/installations` validates the id and returns 501. It stores
  nothing.
- `POST /api/v1/assistant/issues` returns a deterministic draft from the
  snapshot. `archive.stored` is always false. The GitHub App is not configured,
  so the draft is not filed.
- The full debug archive (logcat, dmesg, raw shell sections) is not part of
  the snapshot and is not accepted by these routes. A later upload would be a
  separate consent and a private store; the issue would carry a link, not the
  bytes.
- Settings are not changed. A later write would need an explicit user confirm.
- The HTTP API has no authentication yet. Do not expose the process.

## Run

```sh
VPNHIDE_AI_CHECKOUTS=/var/lib/vpnhide/checkouts \
VPNHIDE_AI_BIND=127.0.0.1:8080 \
cargo run -p vpnhide_ai_support
```

`VPNHIDE_AI_CHECKOUTS` is a directory of source trees, one per app version the
server is willing to answer for:

```text
checkouts/
  1.3.1/     # that release, including docs/
  1.3.2/
```

The directory name is the `version` string on the request. It is a single path
segment (`1.3.1`, `1.3.1-dev`). `..` is rejected.

## Endpoints

| Method | Path | Sketch behavior |
|---|---|---|
| `POST` | `/api/v1/assistant/chat` | Validates version, snapshot, and question. Returns `model_not_configured`, the checkout state (`ready`, `missing`, `unavailable`), and `system_prompt_id`. `answer` is null. |
| `POST` | `/api/v1/installations` | 501 `not_implemented`. Echoes `installation_id`. |
| `POST` | `/api/v1/assistant/issues` | 200 draft title and body. `github.status` is `not_configured`. `archive.stored` is false. |

The chat body is `version`, `snapshot`, `question`, and an optional `locale`.
`version` selects the checkout. It is not taken from `snapshot.app.version`;
when they differ, the issue draft says so.

Body cap is 256 KiB. That is enough for the compact snapshot and too small for
a debug bundle, on purpose.

## Snapshot

Assistant schema `1` (`ASSISTANT_SNAPSHOT_SCHEMA`). It is not
`VPNHIDE_STATE_SCHEMA`. Wire names are snake_case. A verdict is rejected unless
`gate` is `routed`.

| Snapshot field | Taken from |
|---|---|
| `bundle_schema` | `VpnHideState.schema` |
| `generated_at` | `VpnHideState.generatedAt` |
| `capture_kind` | `VpnHideState.captureKind` (`debug`, `full_system_logcat`, `kernel_images`) |
| `app`, `device` | `VpnHideState.app`, `VpnHideState.device` |
| `self_needs_restart` | `VpnHideState.selfNeedsRestart` |
| `gate` | `VpnHideState.gate` / `DiagnosticReport.gate` |
| `self_test_run_id` | `VpnHideState.selfTestRunId` |
| `diagnostics` | `DiagnosticSummaryInfo`: eligibility, run ids, measurement timestamps, applicability, evidence counts and conclusion, `currentSuccess`, `probeUnavailable`, last known routing |
| `native_verdict`, `java_verdict` | `VpnHideState.nativeVerdict`, `VpnHideState.javaVerdict` |
| `report` | `DiagnosticReport`: gate, `complete`, both layers' status, `unownedLeaks`, and checks |
| `report.*.checks` | `DiagnosticCheck.id`, layer, `CheckOutcome`, `owned`, `missingHooks`. Labels and probe sentences stay in the archive |
| `backends`, `active_backend`, `ports` | `NativeBackendStates`, `DisplayNativeBackend`, `VpnHideState.ports` (`ModuleState`) |
| `kmod_load` | `KmodLoadStatus` without `dmesgTail` and `insmodStderr` |
| `root_shell` | `RootShellDiag.uid`, `errnoCtl`, `runtimeCheckable` |
| `errors` | `VpnHideState.errors`, capped |

`CheckOutcome` kinds: `leak`, `hidden_backend`, `hidden_selinux`,
`nothing_to_leak`, `not_measured` (`no_network`, `no_ground_truth`,
`unknown_native_status`).

## Tools

Server-side only. There is no client route that invokes them.

| Tool | Reads | Cap |
|---|---|---|
| `search_docs` | `<checkout>/docs`, including `docs/help/` | query length, match count, snippet bytes, response bytes, files scanned |
| `search_code` | the checkout, skipping `docs/`, `.git`, and build trees | same |
| `read_file` | one file, `offset` (1-based line) and `limit` | line count and bytes; absolute paths, `..`, and symlinks that leave the checkout are errors |
| `search_issues` | nothing | query length; status `not_configured` |

Queries are literal substrings, not shell patterns.

## Prompt

`prompts/system.md`, id `ai-support-draft-1`, compiled into the binary. The
model is told to explain snapshot and checkout facts, to treat a non-`routed`
gate as "not measured", and not to declare protection working without that
measurement. It is told it has no shell.
