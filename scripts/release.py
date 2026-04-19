#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "rich",
# ]
# ///
"""Cut a new release: rotate the unreleased changelog into history and
propagate the new version number to every version-bearing source file.

Usage:
  release.py X.Y.Z

What it does, atomically:
  * `changelog.json`: move `unreleased` -> `history[0]` with
    `version=X.Y.Z`, then reset `unreleased` to empty.
  * Regenerate `CHANGELOG.md` and `update-json/changelog.md`.
  * Write `X.Y.Z` into the `VERSION` file.
  * Patch the pinned version in:
      - `{kmod,zygisk,portshide}/module/module.prop` (version, versionCode)
      - `zygisk/Cargo.toml`                            (first `version = "..."`)
      - `lsposed/native/Cargo.toml`                    (first `version = "..."`)
      - `lsposed/app/build.gradle.kts`                 (versionName, versionCode)

`versionCode` is derived as `major*10000 + minor*100 + patch`.

After this script succeeds:
  1. `git commit -am "chore: release vX.Y.Z"`
  2. `git tag vX.Y.Z && git push && git push origin vX.Y.Z`
  3. Wait for CI to build and publish the GitHub release.
  4. `./scripts/update-json.sh` (post-release step).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from changelog_lib import (  # type: ignore[import-not-found]
    REPO_ROOT,
    load_fragments,
    load_json,
    rotate_fragments_into_history,
    save_json,
    write_md,
)
from rich.console import Console

VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")


def parse_version(raw: str) -> tuple[str, int]:
    if not VERSION_RE.match(raw):
        raise SystemExit(f"error: expected MAJOR.MINOR.PATCH, got {raw!r}")
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


def write_version_file(version: str) -> None:
    (REPO_ROOT / "VERSION").write_text(f"{version}\n", encoding="utf-8")


def main() -> int:
    console = Console()
    if len(sys.argv) != 2:
        console.print("[red]usage:[/red] release.py X.Y.Z")
        return 2

    version, version_code = parse_version(sys.argv[1])
    console.print(f"[bold]Releasing v{version}[/bold]  [dim](versionCode {version_code})[/dim]")

    # Check that the version hasn't already been released.
    data = load_json()
    for past in data.get("history", []):
        if past.get("version") == version:
            console.print(
                f"[red]error:[/red] v{version} already exists in history[]. "
                "Pick a new version.",
            )
            return 1

    fragments = load_fragments()
    if not fragments:
        console.print(
            "[yellow]warning:[/yellow] no changelog fragments under "
            "changelog.d/ — releasing an empty changelog.",
        )

    # Source files must all exist.
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

    # Changelog: rotate fragments into history, then delete them.
    rotate_fragments_into_history(data, fragments, version)
    save_json(data)
    write_md(data)
    console.print(
        f"  [green]✓[/green] changelog: {len(fragments)} fragment(s) → history[0] as v{version}",
    )

    # VERSION file.
    write_version_file(version)
    console.print("  [green]✓[/green] VERSION")

    # Version-bearing source files.
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

    console.print()
    console.print("[bold]Next steps:[/bold]")
    console.print(f"  git commit -am \"chore: release v{version}\"")
    console.print(f"  git tag v{version} && git push && git push origin v{version}")
    console.print("  # CI builds artifacts and creates a DRAFT release — review on the Releases page, click Publish")
    console.print("  ./scripts/update-json.sh")
    console.print(f"  git commit -am \"chore: update-json for v{version}\"")
    console.print("  git push")
    return 0


if __name__ == "__main__":
    sys.exit(main())
