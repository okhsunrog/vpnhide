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
