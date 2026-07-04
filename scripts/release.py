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
      - `lsposed/app/build.gradle.kts`                 (versionCode; versionName
        is derived from `build-version.py`/the git tag, not patched)

`versionCode` is derived as `major*10000 + minor*100 + patch`.

After this script succeeds:
  1. `git commit -am "chore: release vX.Y.Z"`
  2. `git tag vX.Y.Z && git push && git push origin vX.Y.Z`
  3. Wait for CI to build and publish the GitHub release.
  4. `./scripts/update-json.sh` (post-release step).
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from changelog_lib import (  # type: ignore[import-not-found]
    REPO_ROOT,
    delete_fragment_files,
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


def patch_file(
    path: Path,
    replacements: list[tuple[re.Pattern[str], str]],
    *,
    dry_run: bool = False,
) -> None:
    """Apply each pattern → replacement once.

    Hard-fails if any pattern doesn't match. Silently leaving a stale
    version in some file because the format drifted from what the regex
    expects is exactly the failure mode we want to catch loudly.

    With ``dry_run=True`` the patterns are still validated (and a miss still
    raises) but nothing is written — used to pre-flight every source patch
    before the irreversible changelog rotation.
    """
    text = path.read_text(encoding="utf-8")
    new_text = text
    for pattern, replacement in replacements:
        new_text, n = pattern.subn(replacement, new_text, count=1)
        if n == 0:
            raise SystemExit(
                f"error: pattern {pattern.pattern!r} did not match in {path}. "
                f"File format probably changed — update release.py."
            )
    if not dry_run and new_text != text:
        path.write_text(new_text, encoding="utf-8")


def update_module_prop(
    path: Path, version: str, version_code: int, *, dry_run: bool = False
) -> None:
    patch_file(
        path,
        [
            (re.compile(r"^version=.*$", re.M), f"version=v{version}"),
            (re.compile(r"^versionCode=.*$", re.M), f"versionCode={version_code}"),
        ],
        dry_run=dry_run,
    )


def update_cargo_toml(path: Path, version: str, *, dry_run: bool = False) -> None:
    """Replace the first `version = "..."` line — package version sits at top."""
    patch_file(
        path,
        [(re.compile(r'^version = "[^"]*"$', re.M), f'version = "{version}"')],
        dry_run=dry_run,
    )


def update_gradle_kts(path: Path, version_code: int, *, dry_run: bool = False) -> None:
    # Only versionCode is a literal here. versionName is computed at configure
    # time from build-version.py (the git tag `vX.Y.Z` → "X.Y.Z", falling back
    # to the VERSION file), so there is nothing to patch for it.
    patch_file(
        path,
        [
            (re.compile(r"versionCode = \d+"), f"versionCode = {version_code}"),
        ],
        dry_run=dry_run,
    )


def patch_all_sources(version: str, version_code: int, *, dry_run: bool) -> None:
    """Patch (or, with dry_run, just validate) every version-bearing source file."""
    vc = version_code  # local alias to keep the patch calls within the line limit
    update_module_prop(REPO_ROOT / "kmod/module/module.prop", version, vc, dry_run=dry_run)
    update_module_prop(REPO_ROOT / "kmod/kpm/module/module.prop", version, vc, dry_run=dry_run)
    update_module_prop(REPO_ROOT / "zygisk/module/module.prop", version, vc, dry_run=dry_run)
    update_module_prop(REPO_ROOT / "portshide/module/module.prop", version, vc, dry_run=dry_run)
    update_cargo_toml(REPO_ROOT / "crates/protocol/Cargo.toml", version, dry_run=dry_run)
    update_cargo_toml(REPO_ROOT / "crates/activator/Cargo.toml", version, dry_run=dry_run)
    update_cargo_toml(REPO_ROOT / "zygisk/Cargo.toml", version, dry_run=dry_run)
    update_cargo_toml(REPO_ROOT / "lsposed/native/Cargo.toml", version, dry_run=dry_run)
    update_gradle_kts(REPO_ROOT / "lsposed/app/build.gradle.kts", vc, dry_run=dry_run)


def sync_cargo_locks() -> None:
    """Refresh the workspace members' versions in the committed Cargo.lock files.

    We just rewrote the crates' `version = "..."` in Cargo.toml. Without also
    updating Cargo.lock, the first `cargo build` in CI rewrites the lock, which
    dirties the working tree — so `git describe --dirty` stamps every artifact
    "X.Y.Z-dirty". `cargo update --offline --workspace` only bumps the workspace
    members' own versions (no registry access, no dependency churn).
    """
    for lock_dir in (REPO_ROOT, REPO_ROOT / "lsposed" / "native"):
        subprocess.run(
            ["cargo", "update", "--offline", "--workspace"],
            cwd=lock_dir,
            check=True,
            capture_output=True,
            text=True,
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
                f"[red]error:[/red] v{version} already exists in history[]. Pick a new version.",
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
        REPO_ROOT / "kmod/kpm/module/module.prop",
        REPO_ROOT / "zygisk/module/module.prop",
        REPO_ROOT / "portshide/module/module.prop",
        REPO_ROOT / "crates/protocol/Cargo.toml",
        REPO_ROOT / "crates/activator/Cargo.toml",
        REPO_ROOT / "zygisk/Cargo.toml",
        REPO_ROOT / "lsposed/app/build.gradle.kts",
        REPO_ROOT / "lsposed/native/Cargo.toml",
    ]
    for f in files:
        if not f.exists():
            console.print(f"[red]missing:[/red] {f.relative_to(REPO_ROOT)}")
            return 1

    # Pre-flight: validate every version-patch regex BEFORE the irreversible
    # changelog rotation. patch_file hard-fails on a drifted module.prop /
    # Cargo.toml / gradle format; if that failure happened after save_json +
    # delete_fragment_files, changelog.json would already hold the release and
    # the fragments would be gone, while the duplicate-version guard above then
    # refuses any retry. Catching it here leaves the changelog fully retryable.
    patch_all_sources(version, version_code, dry_run=True)

    # Changelog: rotate fragments into history, persist, then delete the
    # fragment files. Now safe — the source patches are known to match.
    rotate_fragments_into_history(data, fragments, version)
    save_json(data)
    write_md(data)
    delete_fragment_files(fragments)
    console.print(
        f"  [green]✓[/green] changelog: {len(fragments)} fragment(s) → history[0] as v{version}",
    )

    # VERSION file.
    write_version_file(version)
    console.print("  [green]✓[/green] VERSION")

    # Version-bearing source files (validated above, so these will not fail).
    patch_all_sources(version, version_code, dry_run=False)

    console.print("  [green]✓[/green] kmod/module/module.prop")
    console.print("  [green]✓[/green] zygisk/module/module.prop")
    console.print("  [green]✓[/green] portshide/module/module.prop")
    console.print("  [green]✓[/green] zygisk/Cargo.toml")
    console.print("  [green]✓[/green] lsposed/native/Cargo.toml")
    console.print("  [green]✓[/green] lsposed/app/build.gradle.kts")

    # Keep Cargo.lock in step with the bumped Cargo.toml versions so the release
    # build starts from a clean tree (otherwise every artifact gets "-dirty").
    sync_cargo_locks()
    console.print("  [green]✓[/green] Cargo.lock (+ lsposed/native)")

    console.print()
    console.print("[bold]Next steps:[/bold]")
    console.print(f'  git commit -am "chore: release v{version}"')
    console.print(f"  git tag v{version} && git push && git push origin v{version}")
    console.print(
        "  # CI builds artifacts and creates a DRAFT release — "
        "review on the Releases page, click Publish"
    )
    console.print("  ./scripts/update-json.sh")
    console.print(f'  git commit -am "chore: update-json for v{version}"')
    console.print("  git push")
    return 0


if __name__ == "__main__":
    sys.exit(main())
