#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "rich",
# ]
# ///
"""Propagate VERSION to all source files and rotate the changelog.

Reads the `VERSION` file (e.g. `0.6.1`) and updates:
  - {kmod,zygisk,portshide}/module/module.prop  (version, versionCode)
  - zygisk/Cargo.toml                            (first `version = "..."`)
  - lsposed/app/build.gradle.kts                 (versionName, versionCode)
  - lsposed/native/Cargo.toml                    (first `version = "..."`)

If the changelog's top-level version differs from VERSION, rotates the
top-level into history[0] and creates a new empty top-level for VERSION.
Then regenerates `update-json/changelog.md` from the JSON source.

Run AFTER editing VERSION; before running ./scripts/update-json.sh
(that one runs after the GitHub release is published).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _changelog import (  # type: ignore[import-not-found]
    REPO_ROOT,
    load_json,
    rotate_for_version,
    save_json,
    write_md,
)
from rich.console import Console

VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")


def read_version() -> tuple[str, int]:
    raw = (REPO_ROOT / "VERSION").read_text(encoding="utf-8").strip()
    if not VERSION_RE.match(raw):
        raise SystemExit(f"error: VERSION must be MAJOR.MINOR.PATCH, got {raw!r}")
    major, minor, patch = (int(p) for p in raw.split("."))
    return raw, major * 10000 + minor * 100 + patch


def patch_file(path: Path, replacements: list[tuple[re.Pattern[str], str]]) -> None:
    text = path.read_text(encoding="utf-8")
    new_text = text
    for pattern, replacement in replacements:
        new_text = pattern.sub(replacement, new_text, count=1)
    if new_text != text:
        path.write_text(new_text, encoding="utf-8")


def update_module_prop(path: Path, version: str, version_code: int) -> None:
    patch_file(
        path,
        [
            (re.compile(r"^version=.*$", re.M), f"version=v{version}"),
            (re.compile(r"^versionCode=.*$", re.M), f"versionCode={version_code}"),
        ],
    )


def update_cargo_toml(path: Path, version: str) -> None:
    """Replace the first `version = "..."` line — package version sits at top."""
    patch_file(
        path,
        [(re.compile(r'^version = "[^"]*"$', re.M), f'version = "{version}"')],
    )


def update_gradle_kts(path: Path, version: str, version_code: int) -> None:
    patch_file(
        path,
        [
            (re.compile(r"versionCode = \d+"), f"versionCode = {version_code}"),
            (re.compile(r'versionName = "[^"]*"'), f'versionName = "{version}"'),
        ],
    )


def main() -> int:
    console = Console()
    version, version_code = read_version()
    console.print(f"[bold]Version:[/bold] {version}  [dim](versionCode {version_code})[/dim]")

    files = [
        REPO_ROOT / "kmod/module/module.prop",
        REPO_ROOT / "zygisk/module/module.prop",
        REPO_ROOT / "portshide/module/module.prop",
        REPO_ROOT / "zygisk/Cargo.toml",
        REPO_ROOT / "lsposed/app/build.gradle.kts",
        REPO_ROOT / "lsposed/native/Cargo.toml",
    ]
    for f in files:
        if not f.exists():
            console.print(f"[red]missing:[/red] {f.relative_to(REPO_ROOT)}")
            return 1

    update_module_prop(REPO_ROOT / "kmod/module/module.prop", version, version_code)
    update_module_prop(REPO_ROOT / "zygisk/module/module.prop", version, version_code)
    update_module_prop(REPO_ROOT / "portshide/module/module.prop", version, version_code)
    update_cargo_toml(REPO_ROOT / "zygisk/Cargo.toml", version)
    update_cargo_toml(REPO_ROOT / "lsposed/native/Cargo.toml", version)
    update_gradle_kts(REPO_ROOT / "lsposed/app/build.gradle.kts", version, version_code)

    console.print("  [green]✓[/green] kmod/module/module.prop")
    console.print("  [green]✓[/green] zygisk/module/module.prop")
    console.print("  [green]✓[/green] portshide/module/module.prop")
    console.print("  [green]✓[/green] zygisk/Cargo.toml")
    console.print("  [green]✓[/green] lsposed/native/Cargo.toml")
    console.print("  [green]✓[/green] lsposed/app/build.gradle.kts")

    data = load_json()
    rotated = rotate_for_version(data, version)
    save_json(data)
    write_md(data)
    if rotated:
        console.print(
            f"  [green]✓[/green] changelog: rotated previous top-level into history, "
            f"new empty top-level for v{version}",
        )
    else:
        console.print(
            f"  [yellow]·[/yellow] changelog: top-level already v{version}, regenerated md",
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
