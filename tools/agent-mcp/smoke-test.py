#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#   "mcp>=1.28,<2",
# ]
# ///
"""End-to-end smoke test for the VPN Hide agent bridge.

Launches the MCP server (`server.py`) over stdio, performs the MCP handshake,
lists the exposed tools, and calls one read-only tool to confirm the full
host -> MCP -> HTTP bridge -> app path works. Requires a debug build with
Settings -> Debugging -> Agent control enabled and the device reachable over adb.

    ./tools/agent-mcp/smoke-test.py --serial 3B241FDJG003LP

Exits non-zero on any failure so it can gate CI/manual checks.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

SERVER = Path(__file__).resolve().parent / "server.py"
PROBE_TOOL = "getState"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Smoke-test the VPN Hide agent bridge over MCP.")
    parser.add_argument("--serial", help="adb device serial to target")
    parser.add_argument("--adb", default="adb", help="adb executable path")
    parser.add_argument(
        "--ui",
        action="store_true",
        help="Also exercise isolated UI scenarios; keep the app foregrounded",
    )
    return parser.parse_args()


async def run(args: argparse.Namespace) -> None:
    server_args = [str(SERVER), "--adb", args.adb]
    if args.serial:
        server_args += ["--serial", args.serial]
    # Launch the server with this interpreter so it shares our resolved env
    # (mcp already installed) instead of nesting another `uv run`, which
    # deadlocks the stdio handshake.
    params = StdioServerParameters(command=sys.executable, args=server_args)
    async with (
        stdio_client(params) as (read, write),
        ClientSession(read, write) as session,
    ):
        await session.initialize()
        tools = await session.list_tools()
        names = sorted(t.name for t in tools.tools)
        print(f"PASS  handshake + tools/list: {len(names)} tools")
        print("      " + ", ".join(names))
        if PROBE_TOOL not in names:
            raise SystemExit(f"FAIL  expected tool {PROBE_TOOL} missing")
        result = await session.call_tool(PROBE_TOOL, {"refresh": False})
        text = result.content[0].text if result.content else ""
        if result.isError or not text:
            raise SystemExit(f"FAIL  {PROBE_TOOL} call errored: {text[:200]}")
        print(f"PASS  tools/call {PROBE_TOOL}: {len(text)} chars returned")
        if args.ui:
            await smoke_ui(session)
    print("OK    agent bridge smoke test passed")


async def ui_call(session: ClientSession, name: str, arguments: dict | None = None):
    result = await session.call_tool(name, arguments or {})
    if result.isError or not result.content:
        raise RuntimeError(f"UI call failed: {name}")
    return json.loads(result.content[0].text)


async def smoke_ui(session: ClientSession) -> None:
    current = await ui_call(session, "getUiScenario")
    if current["scenario"] is not None:
        raise RuntimeError("An interactive preview is already active; exit it before --ui")
    active_id = None
    try:
        catalog = await ui_call(session, "listUiScenarios")
        for spec in catalog:
            started = await ui_call(session, "startUiScenario", {"scenario": spec["id"]})
            active_id = started["sessionId"]
            for _ in range(50):
                observed = await ui_call(session, "getUiScenario")
                if (
                    observed["sessionId"] == active_id
                    and observed["composedRevision"] == started["revision"]
                ):
                    break
                await asyncio.sleep(0.1)
            else:
                raise RuntimeError(
                    f"Scenario did not compose: {spec['id']}; keep the app foregrounded"
                )
            print(f"PASS  UI composed: {spec['id']}")
        navigated = await ui_call(
            session, "navigateUiScenario", {"sessionId": active_id, "destination": "kpm-install"}
        )
        if not navigated["helpOpen"]:
            raise RuntimeError("Preview help navigation failed")
    finally:
        if active_id is not None:
            await ui_call(session, "stopUiScenario", {"sessionId": active_id})
    if (await ui_call(session, "getUiScenario"))["scenario"] is not None:
        raise RuntimeError("Preview did not stop")
    print("PASS  UI preview cleanup")


def main() -> None:
    try:
        asyncio.run(run(parse_args()))
    except SystemExit:
        raise
    except Exception as error:  # noqa: BLE001
        print(f"FAIL  {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
