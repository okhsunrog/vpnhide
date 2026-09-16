#!/usr/bin/env python3
"""Snapshot and compare the post-Binder network view of any uid on a device.

Runs the app's NetworkViewProbeMain through root `app_process` under the uid of
a given package, so a target app's view can be diffed against a non-target's
without installing anything. Stdlib only.

    uv run scripts/network-view-probe.py snapshot --package com.example.bank --expect-hidden
    uv run scripts/network-view-probe.py snapshot --package org.example.plain -o baseline.json
    uv run scripts/network-view-probe.py compare baseline.json target.json
    uv run scripts/network-view-probe.py report target.json

The installed VPN Hide APK provides the probe classes (CLASSPATH); it must be
the build whose sources you are testing. The probe registers network callbacks
for the capture window and unregisters them; it changes nothing on the device.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

APP_PACKAGE = "dev.okhsunrog.vpnhide"
PROBE_CLASS = "dev.okhsunrog.vpnhide.debug.NetworkViewProbeMain"


def adb(serial: str | None, *args: str, timeout: float = 60) -> str:
    cmd = ["adb"] + (["-s", serial] if serial else []) + list(args)
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)
    if result.returncode != 0:
        raise SystemExit(f"{' '.join(cmd)} failed ({result.returncode}):\n{result.stderr}")
    return result.stdout


def package_uid(serial: str | None, package: str) -> int:
    out = adb(serial, "shell", "cmd", "package", "list", "packages", "-U", "--user", "0", package)
    for line in out.splitlines():
        # package:<name> uid:<uid>
        if line.startswith(f"package:{package} "):
            return int(line.rsplit("uid:", 1)[1])
    raise SystemExit(f"package {package} not found for user 0")


def app_apk_path(serial: str | None) -> str:
    out = adb(serial, "shell", "pm", "path", APP_PACKAGE)
    for line in out.splitlines():
        if line.startswith("package:") and line.endswith("base.apk"):
            return line[len("package:") :].strip()
    raise SystemExit(f"{APP_PACKAGE} is not installed (pm path returned: {out!r})")


def snapshot(args: argparse.Namespace) -> int:
    uid = package_uid(args.serial, args.package)
    apk = app_apk_path(args.serial)
    probe_args = [
        "--uid",
        str(uid),
        "--package",
        args.package,
        "--capture-ms",
        str(args.capture_ms),
        "--scan-beyond",
        str(args.scan_beyond),
    ]
    if args.expect_hidden:
        probe_args.append("--expect-hidden")
    command = f"CLASSPATH={apk} app_process /system/bin {PROBE_CLASS} {' '.join(probe_args)}"
    out = adb(
        args.serial,
        "shell",
        "-T",
        f"su -c '{command}'",
        timeout=args.capture_ms / 1000 * 3 + 60,
    )
    start = out.find("{")
    if start < 0:
        raise SystemExit(f"probe produced no JSON:\n{out}")
    data = json.loads(out[start:])
    output = Path(args.output or f"network-view-{args.package}-{uid}.json")
    output.write_text(json.dumps(data, indent=2, ensure_ascii=False))
    print(f"wrote {output}")
    print_report(data)
    return 0


def print_report(data: dict) -> None:
    print(
        f"uid={data['uid']} package={data['packageName']} sdk={data['sdk']} "
        f"expectHidden={data['expectHidden']} active={data['activeNetwork']} "
        f"all={data['allNetworks']} phantoms={data['phantomNetworks']}"
    )
    for net in data["networks"]:
        caps = net.get("capabilities") or {}
        lp = net.get("linkProperties") or {}
        info = net.get("networkInfo") or {}
        sources = ",".join(net["sources"])
        print(
            f"  net {net['netId']} [{sources}] transports={caps.get('transports')} "
            f"iface={lp.get('interfaceName')} info={info.get('typeName')}/{info.get('state')}"
        )
    legacy = data["legacy"]
    active_info = legacy.get("activeNetworkInfo") or {}
    by_type = ", ".join(
        f"{k}:{(v or {}).get('detailedState')}" for k, v in legacy["byType"].items()
    )
    print(
        f"  legacy: active={active_info.get('typeName')}/{active_info.get('state')} "
        f"byType={{{by_type}}} forType={legacy['networkForType']}"
    )
    for event in data["callbacks"]:
        caps = event.get("capabilities") or {}
        lp = event.get("linkProperties") or {}
        extra = ""
        if caps:
            extra = f" transports={caps.get('transports')}"
        if lp:
            extra = f" iface={lp.get('interfaceName')}"
        where = f"{event['registration']}/{event['event']}"
        print(f"  cb +{event['tMs']}ms {where} net={event['netId']}{extra}")
    if data.get("pendingIntent"):
        print(f"  pendingIntent: {data['pendingIntent']}")
    for inv in data["invariants"]:
        mark = {"ok": "OK ", "violated": "BAD", "not_applicable": "n/a"}[inv["status"]]
        print(f"  {mark} {inv['id']}: {inv['detail']}")
    for err in data["errors"]:
        print(f"  error: {err}")


def report(args: argparse.Namespace) -> int:
    print_report(json.loads(Path(args.file).read_text()))
    return 0


def vpn_net_ids(data: dict) -> set[int]:
    return {
        n["netId"]
        for n in data["networks"]
        if "VPN" in ((n.get("capabilities") or {}).get("transports") or [])
    }


def compare(args: argparse.Namespace) -> int:
    base = json.loads(Path(args.baseline).read_text())
    target = json.loads(Path(args.target).read_text())
    vpn = vpn_net_ids(base)
    print(
        f"baseline uid={base['uid']} target uid={target['uid']} baseline VPN netIds={sorted(vpn)}"
    )
    failures = 0

    def check(name: str, ok: bool, detail: str) -> None:
        nonlocal failures
        print(f"  {'OK ' if ok else 'BAD'} {name}: {detail}")
        failures += 0 if ok else 1

    expected_all = sorted(set(base["allNetworks"]) - vpn)
    check(
        "allNetworks = baseline minus VPN",
        sorted(target["allNetworks"]) == expected_all,
        f"target={sorted(target['allNetworks'])} expected={expected_all}",
    )
    base_active = base["activeNetwork"]
    target_active = target["activeNetwork"]
    if base_active in vpn:
        check(
            "active handle replaced by a listed physical network",
            target_active is not None and target_active in expected_all,
            f"baseline active={base_active} (VPN) target active={target_active}",
        )
    else:
        check(
            "active handle unchanged",
            target_active == base_active,
            f"baseline={base_active} target={target_active}",
        )
    base_facts = {n["netId"]: n for n in base["networks"]}
    for net in target["networks"]:
        b = base_facts.get(net["netId"])
        if b is None or net["netId"] in vpn:
            continue
        for key in ("capabilities", "linkProperties"):
            check(
                f"net {net['netId']} {key} equals baseline",
                net.get(key) == b.get(key),
                "same"
                if net.get(key) == b.get(key)
                else f"target={net.get(key)}\n      base={b.get(key)}",
            )
        # NetworkInfo connection state (state/detailedState/available/extraInfo)
        # is the platform's per-uid blocked policy — a foregrounded uid reads
        # CONNECTED where a backgrounded one reads DISCONNECTED/BLOCKED — so
        # comparing it across two uids is not a hiding check. Compare only the
        # network identity (type); a state difference is reported as info.
        ti, bi = net.get("networkInfo") or {}, b.get("networkInfo") or {}
        identity = ("type", "typeName", "subtype")
        t_id = {k: ti.get(k) for k in identity}
        b_id = {k: bi.get(k) for k in identity}
        check(
            f"net {net['netId']} networkInfo type equals baseline",
            t_id == b_id,
            f"target={t_id} base={b_id}",
        )
        if ti.get("state") != bi.get("state"):
            print(
                f"  ... net {net['netId']} state differs (target={ti.get('state')} "
                f"base={bi.get('state')}) — per-uid blocked policy, not a hiding check"
            )
    for inv in target["invariants"]:
        if inv["status"] == "violated":
            check(f"invariant {inv['id']}", False, inv["detail"])
    print(f"{failures} failure(s)")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = parser.add_subparsers(dest="command", required=True)

    snap = sub.add_parser("snapshot", help="take one uid's view on the device")
    snap.add_argument("--package", required=True, help="package whose uid the probe runs as")
    snap.add_argument("-s", "--serial", help="adb device serial")
    snap.add_argument("--capture-ms", type=int, default=2000, help="callback capture window")
    snap.add_argument(
        "--scan-beyond", type=int, default=30, help="netId scan range past the highest listed"
    )
    snap.add_argument(
        "--expect-hidden", action="store_true", help="evaluate the VPN-absence invariants"
    )
    snap.add_argument("-o", "--output", help="where to write the JSON")
    snap.set_defaults(func=snapshot)

    rep = sub.add_parser("report", help="print a saved snapshot")
    rep.add_argument("file")
    rep.set_defaults(func=report)

    cmp_ = sub.add_parser("compare", help="diff a target snapshot against a non-target baseline")
    cmp_.add_argument("baseline")
    cmp_.add_argument("target")
    cmp_.set_defaults(func=compare)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
