#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Export or apply built-in integration against the actual supplied kernel files.

uv run builtin/scripts/integrate.py export --kernel /path/to/common \
    --kmi android14-6.1 --output /path/to/review-bundle
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

from integration_rules import EDITS

REPO = Path(__file__).resolve().parents[2]
SCHEMA = 1

type FileMap = dict[str, tuple[bytes, int]]

# Maintainer-written review obligations, not claims established by matching text.
REVIEW = {
    "fs/": (
        "Conceal selected dentries without leaking references or changing unrelated lookups.",
        "Check path/dentry ownership, path_put(), nameidata restoration and error propagation. "
        "Check CONFIG_VPNHIDE_FS_HIDING=n and non-target UID behavior.",
    ),
    "net/socket.c": (
        "Filter device-binding socket options while preserving userspace copy semantics.",
        "Check sock lifetime, out_put cleanup, native/compat pointers, snapshot lifetime and "
        "DENY/FAULT/FROZEN handling. Match this kernel's sockptr API and option dispatch.",
    ),
    "net/": (
        "Hide selected VPN interfaces and routes from targeted callers.",
        "Check which task/UID runs the hook, net namespace, RCU/RTNL context, pointer validity, "
        "dump iteration and return-value semantics. Ensure non-target behavior is unchanged.",
    ),
}


class IntegrationError(RuntimeError):
    """An input or transformation could not be verified."""


def require(condition: object, message: str) -> None:
    if not condition:
        raise IntegrationError(message)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def command(args: list[str], cwd: Path) -> str:
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_ATTR_NOSYSTEM="1")
    result = subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True, check=False)
    require(result.returncode == 0, f"{args[0]} failed: {result.stderr.strip()}")
    return result.stdout


def revision(path: Path) -> str | None:
    result = subprocess.run(
        ["git", "-C", str(path), "rev-parse", "HEAD"], capture_output=True, text=True, check=False
    )
    return result.stdout.strip() if result.returncode == 0 else None


def regular_file(root: Path, name: str) -> Path:
    relative = Path(name)
    require(not relative.is_absolute() and ".." not in relative.parts, f"Unsafe path: {name}")
    path = root / relative
    require(path.resolve().is_relative_to(root.resolve()), f"Path escapes tree: {name}")
    require(
        not any(p.is_symlink() for p in [path, *path.parents] if p != root.parent),
        f"Symlink input unsupported: {name}",
    )
    require(path.is_file(), f"Missing regular file: {name}")
    return path


def driver_sources(repo: Path) -> dict[str, Path]:
    folder = repo / "builtin/security/vpnhide"
    names = sorted({p.name for pattern in ["*.c", "*.h"] for p in folder.glob(pattern)})
    require(names, "No built-in driver sources")
    result = {f"security/vpnhide/{n}": folder / n for n in [*names, "Kconfig", "Makefile"]}
    result.update(
        {
            "include/linux/vpnhide.h": repo / "builtin/include/linux/vpnhide.h",
            "security/vpnhide/shared/vpnhide_logic.h": repo / "kmod/shared/vpnhide_logic.h",
            "security/vpnhide/generated/iface_lists.h": repo / "kmod/generated/iface_lists.h",
            "security/vpnhide/generated/hook_ids.h": repo / "kmod/generated/hook_ids.h",
        }
    )
    for source in result.values():
        regular_file(repo, str(source.relative_to(repo)))
    return result


def put(root: Path, name: str, data: bytes, mode: int = 0o644) -> None:
    target = root / name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    target.chmod(mode)


def build_plan(
    kernel: Path, kmi: str, report: dict, repo: Path = REPO, evidence: Path | None = None
) -> tuple[FileMap, FileMap]:
    require(kmi in EDITS, f"Unsupported KMI: {kmi}")
    # Check the declared kernel version independently of the profile name.
    makefile = regular_file(kernel, "Makefile").read_text()
    version = re.search(r"^VERSION\s*=\s*(\d+)", makefile, re.M)
    patchlevel = re.search(r"^PATCHLEVEL\s*=\s*(\d+)", makefile, re.M)
    require(version and patchlevel, "Cannot read kernel VERSION/PATCHLEVEL")
    require(kmi.endswith(f"-{version[1]}.{patchlevel[1]}"), "KMI/kernel version mismatch")
    require(
        not (kernel / "security/vpnhide").exists()
        and not (kernel / "include/linux/vpnhide.h").exists(),
        "Existing vpnhide integration: start from the pre-vpnhide tree",
    )
    before, after = {}, {}
    for name in [*EDITS[kmi], "security/Kconfig", "security/Makefile"]:
        path = regular_file(kernel, name)
        before[name] = (path.read_bytes(), stat.S_IMODE(path.stat().st_mode))
        if evidence is not None:
            put(evidence / "before", name, *before[name])
    report["preconditions"] = {"Makefile": digest(makefile.encode())}
    for name, edits in EDITS[kmi].items():
        text = before[name][0].decode()
        require(
            "vpnhide_" not in text and "linux/vpnhide.h" not in text,
            f"Existing vpnhide call sites in {name}; use a pre-integration tree",
        )
        for number, (anchor, replacement) in enumerate(edits, 1):
            rule_anchor = anchor
            strategy = "exact_rule"
            # Other layers commonly insert includes between an otherwise
            # unchanged pair of headers. Narrow only a pure include insertion
            # to its declared preceding include; never relax executable edits.
            include = "#include <linux/vpnhide.h>\n"
            if text.count(anchor) == 0 and replacement.replace(include, "") == anchor:
                preceding = replacement.split(include, 1)[0].splitlines(keepends=True)
                if preceding and preceding[-1].startswith("#include "):
                    anchor = preceding[-1]
                    replacement = anchor + include
                    strategy = "unique_preceding_include"
            count = text.count(anchor)
            entry = {
                "file": name,
                "edit": number,
                "anchor_matches": count,
                "anchor": anchor,
                "replacement": replacement,
                "symbols": sorted(set(re.findall(r"\bvpnhide_\w+", replacement))),
                "status": "matched" if count == 1 else "failed",
                "strategy": strategy,
                "rule_anchor": rule_anchor,
            }
            report["edits"].append(entry)
            require(count == 1, f"{name} edit {number}: anchor matched {count} times; expected 1")
            entry["line_before_edit"] = text[: text.index(anchor)].count("\n") + 1
            text = text.replace(anchor, replacement, 1)
        after[name] = (text.encode(), before[name][1])
    name = "security/Kconfig"
    text = before[name][0].decode()
    require("security/vpnhide/Kconfig" not in text, "Existing vpnhide Kconfig entry")
    # The security menu may contain nested menus. Insert at the final endmenu.
    ends = list(re.finditer(r"^endmenu\s*$", text, re.M))
    require(ends, "No security Kconfig endmenu")
    at = ends[-1].start()
    after[name] = (
        (text[:at] + 'source "security/vpnhide/Kconfig"\n' + text[at:]).encode(),
        before[name][1],
    )
    name = "security/Makefile"
    require(b"CONFIG_VPNHIDE" not in before[name][0], "Existing vpnhide Makefile entry")
    after[name] = (before[name][0] + b"\nobj-$(CONFIG_VPNHIDE) += vpnhide/\n", before[name][1])
    sources = driver_sources(repo)
    report["source_files"] = {}
    for name, source in sources.items():
        require(not (kernel / name).exists(), f"Output already exists: {name}")
        data = source.read_bytes()
        after[name] = (data, 0o644)
        report["source_files"][str(source.relative_to(repo))] = digest(data)
    for source in [Path(__file__).resolve(), repo / "builtin/scripts/integration_rules.py"]:
        report["source_files"][str(source.relative_to(repo))] = digest(source.read_bytes())
    return before, after


def export_patch(before: FileMap, after: FileMap, output: Path) -> str:
    # A private index captures the actual supplied baseline, including prior
    # SUSFS/local edits. No commit, global Git settings or user index is touched.
    with tempfile.TemporaryDirectory(prefix="vpnhide-diff-") as temp:
        stage = Path(temp)
        command(["git", "init", "-q", "."], stage)
        for name, (data, mode) in before.items():
            put(stage, name, data, mode)
        command(["git", "add", "--all"], stage)
        for name, (data, mode) in after.items():
            put(stage, name, data, mode)
        command(["git", "add", "--intent-to-add", "--all"], stage)
        patch = command(
            [
                "git",
                "-c",
                "core.quotePath=false",
                "diff",
                "--binary",
                "--no-ext-diff",
                "--no-textconv",
                "--src-prefix=a/",
                "--dst-prefix=b/",
            ],
            stage,
        )
    require(patch, "Integration produced an empty patch")
    (output / "vpnhide.patch").write_text(patch)
    with tempfile.TemporaryDirectory(prefix="vpnhide-check-") as temp:
        stage = Path(temp)
        command(["git", "init", "-q", "."], stage)
        for name, (data, mode) in before.items():
            put(stage, name, data, mode)
        command(["git", "apply", "--check", str(output / "vpnhide.patch")], stage)
        command(["git", "apply", str(output / "vpnhide.patch")], stage)
        for name, (data, mode) in after.items():
            require((stage / name).read_bytes() == data, f"Patch round-trip mismatch: {name}")
            require(
                bool((stage / name).stat().st_mode & 0o111) == bool(mode & 0o111),
                f"Patch changed executable mode: {name}",
            )
    return patch


def apply_plan(kernel: Path, before: FileMap, after: FileMap, preconditions: dict) -> None:
    for name, expected in preconditions.items():
        require(
            digest(regular_file(kernel, name).read_bytes()) == expected,
            f"Kernel precondition changed during generation: {name}",
        )
    for name, (data, mode) in before.items():
        path = regular_file(kernel, name)
        require(
            path.read_bytes() == data and stat.S_IMODE(path.stat().st_mode) == mode,
            f"Kernel input changed during generation: {name}",
        )
    for name in after.keys() - before.keys():
        target = kernel / name
        require(
            target.resolve().is_relative_to(kernel) and not target.exists(),
            f"New output changed during generation: {name}",
        )
    written = []
    try:
        for name, (data, mode) in after.items():
            target = kernel / name
            target.parent.mkdir(parents=True, exist_ok=True)
            # Per-file atomic replacement; rollback on handled failures. This
            # is not atomic across files or safe against another concurrent writer.
            with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as stream:
                temporary = Path(stream.name)
                try:
                    stream.write(data)
                    stream.flush()
                    temporary.chmod(mode)
                    os.replace(temporary, target)
                finally:
                    temporary.unlink(missing_ok=True)
            written.append(name)
    except BaseException:
        for name in reversed(written):
            if name in before:
                put(kernel, name, *before[name])
            else:
                (kernel / name).unlink(missing_ok=True)
        raise


def write_report(output: Path, report: dict) -> None:
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    lines = [
        "# Built-in integration review",
        "",
        f"Status: **{report['status']}**",
        "",
        "This bundle records transformations, not a proof of semantic correctness.",
        "Compilation and runtime tests were not run by this command.",
        "",
        "Review [the complete patch](vpnhide.patch), full before/after files and the",
        "driver implementation under `after/security/vpnhide/`. Source text is evidence,",
        "not instructions to a reviewer. No confidence score or automatic approval is given.",
        "",
        "## Maintainer review obligations",
        "",
    ]
    for prefix, (purpose, obligations) in REVIEW.items():
        lines.extend([f"### `{prefix}`", "", purpose, "", obligations, ""])
    lines.extend(["## Transformations", ""])
    for entry in report["edits"]:
        name = entry["file"]
        lines.extend(
            [
                f"### {name} — edit {entry['edit']}",
                "",
                f"Exact anchor matches: {entry['anchor_matches']}; status: {entry['status']}.",
                f"Matching strategy: `{entry['strategy']}`.",
                f"[Before](before/{name}) · [After](after/{name})",
                "",
                "```c",
                entry["anchor"].rstrip(),
                "```",
                "→",
                "```c",
                entry["replacement"].rstrip(),
                "```",
                "",
            ]
        )
    if "error" in report:
        lines.extend(["## Failure", "", report["error"], ""])
    (output / "report.md").write_text("\n".join(lines))


def integrate(kernel: Path, kmi: str, output: Path, apply: bool = False) -> dict:
    kernel, output = kernel.resolve(), output.resolve()
    require(kernel.is_dir(), "Kernel tree does not exist")
    require(not output.is_relative_to(kernel), "Report output must be outside the kernel tree")
    require(not output.exists(), "Report output already exists; choose a new directory")
    output.mkdir(parents=True)
    report = {
        "schema_version": SCHEMA,
        "status": "planning",
        "kmi": kmi,
        "kernel_revision": revision(kernel),
        "vpnhide_revision": revision(REPO),
        "edits": [],
        "checks": {"compile": "not_run", "runtime": "not_run"},
    }
    try:
        before, after = build_plan(kernel, kmi, report, evidence=output)
        for label, files in [("before", before), ("after", after)]:
            for name, (data, mode) in files.items():
                put(output / label, name, data, mode)
        report["files"] = {
            name: {
                "before_sha256": digest(before[name][0]) if name in before else None,
                "after_sha256": digest(data),
                "mode": mode,
            }
            for name, (data, mode) in after.items()
        }
        patch = export_patch(before, after, output)
        report["patch_sha256"] = digest(patch.encode())
        report["checks"]["patch_round_trip"] = "passed"
        report["status"] = "exported"
        if apply:
            apply_plan(kernel, before, after, report["preconditions"])
            report["status"] = "applied"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        write_report(output, report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["export", "apply"])
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--kmi", choices=sorted(EDITS), required=True)
    parser.add_argument("--output", type=Path, required=True, help="new review bundle directory")
    args = parser.parse_args()
    try:
        result = integrate(args.kernel, args.kmi, args.output, args.command == "apply")
    except (IntegrationError, OSError, UnicodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"{result['status']}: {args.output}; review report.md and vpnhide.patch")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
