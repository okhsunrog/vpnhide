#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = [
#   "mcp>=1.28,<2",
# ]
# ///

from __future__ import annotations

import argparse
import asyncio
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

DEFAULT_PACKAGE = "dev.okhsunrog.vpnhide"
DEFAULT_HOST_PORT = 27193
DEFAULT_DEVICE_PORT = 27193
TOKEN_RE = re.compile(r"token=([A-Za-z0-9_-]+)")


@dataclass(frozen=True)
class BridgeConfig:
    adb: str
    serial: str | None
    package: str
    host_port: int
    device_port: int


class BridgeClient:
    def __init__(self, config: BridgeConfig) -> None:
        self.config = config
        self.token: str | None = None

    def connect(self) -> None:
        self._adb("forward", f"tcp:{self.config.host_port}", f"tcp:{self.config.device_port}")
        self.token = self._read_token()

    def functions(self) -> list[dict[str, Any]]:
        payload = self._request("GET", "/functions")
        functions = payload.get("functions")
        if not isinstance(functions, list):
            raise RuntimeError("Bridge returned an invalid /functions response")
        return functions

    def call(self, name: str, args: dict[str, Any]) -> Any:
        return self._request("POST", "/call", {"fn": name, "args": args})

    def _request(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
    ) -> Any:
        if self.token is None:
            raise RuntimeError("Bridge token is not loaded")
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.config.host_port}{path}",
            data=data,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                raw = response.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(
                f"Bridge {method} {path} failed: HTTP {error.code}: {detail}"
            ) from error
        except OSError as error:
            raise RuntimeError(
                "Could not reach VPN Hide bridge. Install a debug build, open the app, "
                "and enable Settings -> Debugging -> Agent control.",
            ) from error
        return json.loads(raw) if raw else None

    def _read_token(self) -> str:
        token = self._read_token_file()
        if token:
            return token
        token = self._read_token_logcat()
        if token:
            return token
        raise RuntimeError(
            "Could not read bridge token. Enable Agent control in the debug app, "
            "then retry. Expected files/agent_bridge_token via run-as.",
        )

    def _read_token_file(self) -> str | None:
        result = self._adb_result(
            "shell", "run-as", self.config.package, "cat", "files/agent_bridge_token"
        )
        if result.returncode != 0:
            return None
        token = result.stdout.strip()
        return token or None

    def _read_token_logcat(self) -> str | None:
        result = self._adb_result("logcat", "-d", "-s", "VpnHideAgentBridge")
        if result.returncode != 0:
            return None
        matches = TOKEN_RE.findall(result.stdout)
        return matches[-1] if matches else None

    def _adb(self, *args: str) -> str:
        result = self._adb_result(*args)
        if result.returncode != 0:
            raise RuntimeError(
                result.stderr.strip() or result.stdout.strip() or f"adb {' '.join(args)} failed"
            )
        return result.stdout

    def _adb_result(self, *args: str) -> subprocess.CompletedProcess[str]:
        cmd = [self.config.adb]
        if self.config.serial:
            cmd += ["-s", self.config.serial]
        cmd += list(args)
        return subprocess.run(cmd, text=True, capture_output=True, check=False)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Expose VPN Hide debug bridge as MCP tools.")
    parser.add_argument("--serial", help="adb device serial to target")
    parser.add_argument("--adb", default="adb", help="adb executable path")
    parser.add_argument("--package", default=DEFAULT_PACKAGE, help="VPN Hide application id")
    parser.add_argument(
        "--host-port", type=int, default=DEFAULT_HOST_PORT, help="localhost port for adb forward"
    )
    parser.add_argument(
        "--device-port", type=int, default=DEFAULT_DEVICE_PORT, help="device bridge port"
    )
    return parser.parse_args()


def make_tool(spec: dict[str, Any]) -> Tool:
    return Tool(
        name=spec["name"],
        description=spec.get("description", ""),
        inputSchema=spec.get("inputSchema", {"type": "object", "properties": {}}),
    )


async def main() -> None:
    args = parse_args()
    config = BridgeConfig(
        adb=args.adb,
        serial=args.serial,
        package=args.package,
        host_port=args.host_port,
        device_port=args.device_port,
    )
    client = BridgeClient(config)
    try:
        client.connect()
        function_specs = client.functions()
    except Exception as error:
        print(f"Failed to initialize VPN Hide MCP bridge: {error}", file=sys.stderr)
        raise

    tools_by_name = {spec["name"]: make_tool(spec) for spec in function_specs}
    server = Server("vpnhide-agent-bridge")

    @server.list_tools()
    async def list_tools() -> list[Tool]:
        return list(tools_by_name.values())

    @server.call_tool()
    async def call_tool(name: str, arguments: dict[str, Any] | None) -> list[TextContent]:
        result = await asyncio.to_thread(client.call, name, arguments or {})
        return [TextContent(type="text", text=json.dumps(result, ensure_ascii=False, indent=2))]

    async with stdio_server() as (read_stream, write_stream):
        await server.run(read_stream, write_stream, server.create_initialization_options())


if __name__ == "__main__":
    asyncio.run(main())
