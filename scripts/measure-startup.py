#!/usr/bin/env python3
"""Measure VPN Hide cold startup until the Dashboard is ready.

The script reports both Android's `am start -W` timing and the app-defined
`VpnHide-Startup event=dashboard_ready` marker. The latter is the primary
metric: it fires after the startup splash can be released and the Dashboard
content has reached a Compose frame. All other `VpnHide-Startup` events are
captured as stage timings for profiling startup work.
"""

from __future__ import annotations

import argparse
import re
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass

PACKAGE = "dev.okhsunrog.vpnhide"
ACTIVITY = f"{PACKAGE}/.startup.MainActivity"
STARTUP_TAG = "VpnHide-Startup"


@dataclass(frozen=True)
class Sample:
    total_time_ms: int | None
    wait_time_ms: int | None
    events: dict[str, int]
    metrics: dict[str, int]

    @property
    def dashboard_ready_ms(self) -> int:
        return self.events["dashboard_ready"]

    @property
    def failed(self) -> bool:
        return any(
            event in self.events
            for event in (
                "self_targets_failed",
                "root_snapshot_failed",
                "targets_cache_failed",
                "dashboard_state_failed",
            )
        )


def adb(
    serial: str | None, *args: str, check: bool = True, timeout: float | None = None
) -> subprocess.CompletedProcess[str]:
    cmd = ["adb"]
    if serial:
        cmd += ["-s", serial]
    cmd += list(args)
    return subprocess.run(cmd, check=check, capture_output=True, text=True, timeout=timeout)


def parse_am_time(output: str, key: str) -> int | None:
    match = re.search(rf"^{re.escape(key)}:\s*(\d+)$", output, re.MULTILINE)
    return int(match.group(1)) if match else None


def read_startup_trace(serial: str | None) -> tuple[dict[str, int], dict[str, int]]:
    out = adb(serial, "logcat", "-d", "-v", "brief", "-s", f"{STARTUP_TAG}:I", "*:S").stdout
    events: dict[str, int] = {}
    for event, elapsed_ms in re.findall(r"event=([a-zA-Z0-9_]+) elapsedMs=(\d+)", out):
        events[event] = int(elapsed_ms)
    metrics: dict[str, int] = {}
    for metric, value_ms in re.findall(r"metric=([a-zA-Z0-9_]+) valueMs=(\d+)", out):
        metrics[metric] = int(value_ms)
    return events, metrics


def percentile(values: list[int], pct: float) -> int:
    if not values:
        raise ValueError("empty values")
    ordered = sorted(values)
    index = min(len(ordered) - 1, round((len(ordered) - 1) * pct))
    return ordered[index]


def measure_one(serial: str | None, timeout_sec: float, cold_delay_sec: float) -> Sample:
    adb(serial, "logcat", "-c")
    adb(serial, "shell", "am", "force-stop", PACKAGE)
    time.sleep(cold_delay_sec)

    am = adb(serial, "shell", "am", "start", "-W", "-n", ACTIVITY, timeout=timeout_sec)
    deadline = time.monotonic() + timeout_sec
    events: dict[str, int] = {}
    metrics: dict[str, int] = {}
    while time.monotonic() < deadline:
        events, metrics = read_startup_trace(serial)
        if "dashboard_ready" in events:
            break
        time.sleep(0.1)

    if "dashboard_ready" not in events:
        raise TimeoutError(
            f"did not see {STARTUP_TAG} dashboard_ready marker within {timeout_sec:.1f}s"
        )

    return Sample(
        total_time_ms=parse_am_time(am.stdout, "TotalTime"),
        wait_time_ms=parse_am_time(am.stdout, "WaitTime"),
        events=events,
        metrics=metrics,
    )


def print_summary(name: str, values: list[int]) -> None:
    print(
        f"{name}: median={round(statistics.median(values))}ms "
        f"p90={percentile(values, 0.90)}ms min={min(values)}ms max={max(values)}ms"
    )


def format_event(sample: Sample, event: str) -> str:
    value = sample.events.get(event)
    return f"{event}={value}ms" if value is not None else f"{event}=?"


def format_metric(sample: Sample, metric: str) -> str:
    value = sample.metrics.get(metric)
    return f"{metric}={value}ms" if value is not None else f"{metric}=?"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-s", "--serial", help="adb device serial")
    parser.add_argument("-n", "--runs", type=int, default=10, help="number of cold starts")
    parser.add_argument(
        "--timeout", type=float, default=20.0, help="seconds to wait for each launch"
    )
    parser.add_argument(
        "--cold-delay", type=float, default=1.0, help="seconds after force-stop before launching"
    )
    args = parser.parse_args()

    if args.runs <= 0:
        parser.error("--runs must be positive")

    adb(args.serial, "start-server")
    samples: list[Sample] = []
    run_events = [
        "activity_on_create",
        "self_targets_start",
        "self_targets_done",
        "self_targets_failed",
        "root_snapshot_start",
        "root_snapshot_done",
        "root_snapshot_failed",
        "targets_cache_failed",
        "dashboard_derive_start",
        "dashboard_modules_done",
        "dashboard_kernel_done",
        "dashboard_lsposed_config_done",
        "dashboard_lsposed_done",
        "dashboard_issues_done",
        "dashboard_protection_done",
        "dashboard_state_failed",
        "dashboard_ready",
    ]
    run_metrics = [
        "dashboard_lsposed_db_copy",
        "dashboard_lsposed_db_query",
        "root_shell_module_props",
        "root_shell_target_files",
        "root_shell_kmod_status_files",
        "root_shell_runtime_status_files",
        "root_shell_pm_packages",
        "root_shell_shell_probe_proc_exists",
        "root_shell_shell_probe_ports_chain",
        "root_shell_shell_probe_lsposed_framework",
        "root_shell_shell_probe_vpn_ifaces",
    ]
    for idx in range(1, args.runs + 1):
        sample = measure_one(args.serial, args.timeout, args.cold_delay)
        samples.append(sample)
        print(
            f"run {idx}: dashboardReady={sample.dashboard_ready_ms}ms "
            f"status={'error' if sample.failed else 'ok'} "
            f"amTotal={sample.total_time_ms if sample.total_time_ms is not None else '?'}ms "
            f"amWait={sample.wait_time_ms if sample.wait_time_ms is not None else '?'}ms"
        )
        print(
            "  "
            + " ".join(
                format_event(sample, event) for event in run_events if event in sample.events
            )
        )
        if sample.metrics:
            print(
                "  "
                + " ".join(
                    format_metric(sample, metric)
                    for metric in run_metrics
                    if metric in sample.metrics
                )
            )
        sys.stdout.flush()

    dashboard = [sample.dashboard_ready_ms for sample in samples]
    print_summary("dashboardReady", dashboard)
    failures = sum(1 for sample in samples if sample.failed)
    if failures:
        print(f"startupFailures: {failures}/{len(samples)}")

    total = [sample.total_time_ms for sample in samples if sample.total_time_ms is not None]
    if total:
        print_summary("amTotal", total)

    wait = [sample.wait_time_ms for sample in samples if sample.wait_time_ms is not None]
    if wait:
        print_summary("amWait", wait)

    all_events = sorted({event for sample in samples for event in sample.events})
    if all_events:
        print()
        print("Startup events:")
        for event in all_events:
            values = [sample.events[event] for sample in samples if event in sample.events]
            print_summary(event, values)

    all_metrics = sorted({metric for sample in samples for metric in sample.metrics})
    if all_metrics:
        print()
        print("Startup metrics:")
        for metric in all_metrics:
            values = [sample.metrics[metric] for sample in samples if metric in sample.metrics]
            print_summary(metric, values)

    stage_pairs = [
        ("self_targets", "self_targets_start", "self_targets_done"),
        ("root_snapshot", "root_snapshot_start", "root_snapshot_done"),
        ("derive_modules", "dashboard_derive_start", "dashboard_modules_done"),
        ("kernel", "dashboard_modules_done", "dashboard_kernel_done"),
        ("lsposed_config", "dashboard_kernel_done", "dashboard_lsposed_config_done"),
        ("lsposed_state", "dashboard_lsposed_config_done", "dashboard_lsposed_done"),
        ("protection", "dashboard_protection_start", "dashboard_protection_done"),
        ("issues", "dashboard_protection_done", "dashboard_issues_done"),
        ("compose_frame", "dashboard_issues_done", "dashboard_ready"),
    ]
    print()
    print("Startup stage deltas:")
    for name, start, end in stage_pairs:
        values = [
            sample.events[end] - sample.events[start]
            for sample in samples
            if start in sample.events and end in sample.events
        ]
        if values:
            print_summary(name, values)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
