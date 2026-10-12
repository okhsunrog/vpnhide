# VPN Hide Agent Bridge

Host-side MCP bridge for driving a VPN Hide build through `adb`.

The Android app starts a local HTTP bridge on `127.0.0.1:27193` when
Settings -> Developer -> Agent control is enabled. It is **off by default** and
ships in release builds too. The bridge is loopback-only and requires a bearer
token stored in the app-private file `files/agent_bridge_token`. While it is on,
the dashboard shows a note — leaving the port open is an on-device fingerprint,
so turn it off when you are done.

## Prerequisites

- A VPN Hide APK installed on the target device (debug or release).
- VPN Hide opened at least once.
- Settings -> Developer -> Agent control enabled.
- `adb` can see the device.
- `uv` is installed on the host.

For multi-device setups, pass `--serial <adb-serial>`.

## Raw HTTP

Forward the bridge:

```sh
adb forward tcp:27193 tcp:27193
```

Read the token. On a debug build, `run-as` reaches the app sandbox:

```sh
TOKEN="$(adb shell run-as dev.okhsunrog.vpnhide cat files/agent_bridge_token)"
```

On a release build `run-as` is unavailable, so read the token the bridge logs
once at startup:

```sh
TOKEN="$(adb logcat -d -s VpnHideAgentBridge | sed -n 's/.*token=\([A-Za-z0-9_-]*\).*/\1/p' | tail -1)"
```

List available functions:

```sh
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:27193/functions
```

Call a function:

```sh
curl -s \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"fn":"getStatisticsCaptureBaseline","args":{}}' \
  http://127.0.0.1:27193/call
```

## MCP server

Run directly:

```sh
uv run tools/agent-mcp/server.py --serial 3B241FDJG003LP
```

Example MCP client config:

```json
{
  "mcpServers": {
    "vpnhide": {
      "command": "uv",
      "args": [
        "run",
        "/absolute/path/to/vpnhide/tools/agent-mcp/server.py",
        "--serial",
        "3B241FDJG003LP"
      ]
    }
  }
}
```

The MCP server fetches `/functions` from the app at startup and exposes one MCP
tool per bridge function. Tool calls are forwarded to `POST /call` with the
same bearer token.

## Smoke test

`smoke-test.py` drives the full host -> MCP -> HTTP bridge -> app path: it
launches the server over stdio, performs the MCP handshake, lists tools, and
calls one read-only tool. Use it to confirm a debug build with Agent control
enabled is reachable:

```sh
./tools/agent-mcp/smoke-test.py --serial 3B241FDJG003LP
```

It exits non-zero on any failure.

## UI scenarios

The normal APK includes a process-only preview host behind the existing authenticated,
opt-in bridge. No extra Activity, fixture APK, root-state modification or reboot is
needed. Keep the app in the foreground; the API does not launch an Activity from the
background. The preview is visibly labelled as synthetic and has an Exit button.
It overlays the existing app so navigation and unsaved editor drafts remain intact.

Discover the catalog with `listUiScenarios`, then call:

```json
{"fn":"startUiScenario","args":{"scenario":"kpm-runtime-missing"}}
```

Use the returned `sessionId` for subsequent operations:

```json
{"fn":"getUiScenario","args":{}}
{"fn":"navigateUiScenario","args":{"sessionId":1,"destination":"kpm-install#where-to-download-and-how-to-install-kpatch-next-module"}}
{"fn":"backUiScenario","args":{"sessionId":1}}
{"fn":"stopUiScenario","args":{"sessionId":1}}
```

Anchor slugs must match the current app language. Omit `destination` to open the
contents page. Repeating a navigation request retriggers scrolling and highlighting.
A `help` scenario can also start directly at a destination. The catalog currently
covers KPM/kmod/LSPosed errors, backend conflicts, targets, ports, SELinux, the dashboard
loading skeleton and the offline guide. Each uses production rendering and localized
structured diagnoses. These are focused component scenarios, not fabricated full
root snapshots or simulated diagnostic runs. Add new cases to `AgentUiScenarioData.kt`
and reuse their production composables in the single host.

`getUiScenario` reports requested destination, observed help location, revision,
`composedRevision`, and `lastAction`. Wait for `composedRevision == revision` before UI
automation, then inspect the screen (composition acknowledgement is not proof that
asynchronous article loading or scrolling has completed). Help links navigate through
the real guide; external URLs and actions leading out of the preview are intercepted
and recorded rather than executing configuration changes. `backUiScenario` returns
to the initial issue; on that issue or a standalone help scenario it exits. The guide's
own Back button retains its ordinary article-to-contents behavior.

Starting another scenario replaces the current one. Stale session commands are rejected;
late rendering callbacks are ignored. Rotation keeps the process-owned session; disabling
Agent Control or process death clears it. Nothing is persisted, and `getState` / debug
bundles continue to report the real device rather than the preview. The host MCP server
exposes these functions automatically through the existing `/functions` discovery.

To exercise the discovered catalog through MCP and check preview cleanup:

```sh
uv run tools/agent-mcp/smoke-test.py --serial <adb-serial> --ui
```

This refuses to replace an already active interactive preview. It checks composition
acknowledgement and API lifecycle, not visual correctness; use screenshots and taps
for anchor placement, animation, accessibility and layout acceptance.
