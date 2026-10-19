"""Shared helpers for build scripts.

Used by kmod/build.py, zygisk/build.py,
and scripts/build-version.py.

Stdlib-only on purpose: scripts/build-version.py is invoked from
lsposed/app/build.gradle.kts on every Gradle build, so adding pip/uv
dependencies here would break the APK build for anyone without those
tools available.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import zipfile
from pathlib import Path


def make_zip(source_dir: Path, output_zip: Path) -> None:
    """Create a zip archive from source_dir contents."""
    with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
        for file_path in source_dir.rglob("*"):
            if file_path.is_file():
                arcname = file_path.relative_to(source_dir)
                zf.write(file_path, arcname)


def version_sort_key(name: str) -> tuple[int, ...]:
    """Sort key that orders strings by their embedded integer runs.

    Used to pick the highest version when multiple toolchain directories
    coexist (NDK 25.0.1 vs 100.0.0, clang-r450b vs clang-r498344b) where
    plain lexicographic sort gives the wrong answer.
    """
    return tuple(int(part) for part in re.findall(r"\d+", name))


def get_build_version(repo_root: Path | None = None) -> str:
    """Get the effective build version for vpnhide artifacts.

    - VPNHIDE_BUILD_VERSION set    -> that exact value
    - HEAD on a tag vX.Y.Z        -> "X.Y.Z"          (release build)
    - N commits after tag vX.Y.Z  -> "X.Y.Z-N-gSHA"   (dev build)
    - working tree dirty          -> additional "-dirty" suffix
    - no git / no matching tag    -> falls back to VERSION file
    """
    override = os.environ.get("VPNHIDE_BUILD_VERSION")
    if override and override.strip():
        return override.strip().removeprefix("v")

    if repo_root is None:
        repo_root = Path(__file__).resolve().parent.parent

    repo_root = repo_root.resolve()
    result = subprocess.run(
        [
            "git",
            "-c",
            f"safe.directory={repo_root}",
            "describe",
            "--tags",
            "--match",
            "v*",
            "--dirty",
        ],
        cwd=repo_root,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        return result.stdout.strip().removeprefix("v")

    version_file = repo_root / "VERSION"
    return version_file.read_text(encoding="utf-8").strip()


def detect_android_ndk() -> str | None:
    """Find an Android NDK for cargo-ndk based builds."""
    android_ndk_home = os.environ.get("ANDROID_NDK_HOME")
    if android_ndk_home and Path(android_ndk_home).is_dir():
        return android_ndk_home

    ndk_base = Path.home() / "Android" / "Sdk" / "ndk"
    if not ndk_base.exists():
        return None
    versions = sorted((d.name for d in ndk_base.iterdir() if d.is_dir()), key=version_sort_key)
    if not versions:
        return None
    return str(ndk_base / versions[-1])


