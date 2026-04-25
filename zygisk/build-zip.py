#!/usr/bin/env python3
"""Build the native library for aarch64 Android and package it into an
installable KernelSU/Magisk module zip.

Requirements:
  - rustup target aarch64-linux-android (already installed)
  - cargo-ndk
  - Android NDK at $ANDROID_NDK_HOME or auto-detected from $HOME/Android/Sdk/ndk/*
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
from build_lib import get_build_version, make_zip, version_sort_key  # type: ignore[import-not-found]


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    os.chdir(script_dir)

    # Auto-detect NDK if ANDROID_NDK_HOME isn't set
    android_ndk_home = os.environ.get("ANDROID_NDK_HOME")
    if not android_ndk_home:
        ndk_base = Path.home() / "Android" / "Sdk" / "ndk"
        if ndk_base.exists():
            ndk_versions = sorted(
                (d.name for d in ndk_base.iterdir() if d.is_dir()),
                key=version_sort_key,
            )
            if ndk_versions:
                android_ndk_home = str(ndk_base / ndk_versions[-1])

    if not android_ndk_home or not Path(android_ndk_home).is_dir():
        print(
            "error: ANDROID_NDK_HOME not set and no NDK found under ~/Android/Sdk/ndk",
            file=sys.stderr,
        )
        return 1

    print(f"Using NDK: {android_ndk_home}")
    os.environ["ANDROID_NDK_HOME"] = android_ndk_home

    # Build the cdylib for arm64-v8a
    subprocess.run(
        ["cargo", "ndk", "-t", "arm64-v8a", "build", "--release"],
        check=True,
    )

    so_src = script_dir / "target" / "aarch64-linux-android" / "release" / "libvpnhide_zygisk.so"
    if not so_src.exists():
        print(f"error: expected {so_src} after cargo ndk build, not found", file=sys.stderr)
        return 1

    # Assemble the module staging directory
    staging = script_dir / "target" / "module-staging"
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(script_dir / "module", staging)
    (staging / "zygisk").mkdir(parents=True, exist_ok=True)
    shutil.copy(so_src, staging / "zygisk" / "arm64-v8a.so")

    # Get build version
    build_version = get_build_version(script_dir.parent)

    # Stamp the effective build version into the staging module.prop without
    # touching the committed file. On a release tag this matches VERSION; on
    # any other commit the git suffix makes dev builds identifiable.
    module_prop = staging / "module.prop"
    content = module_prop.read_text(encoding="utf-8")
    content = re.sub(r"^version=.*", f"version=v{build_version}", content, flags=re.MULTILINE)
    module_prop.write_text(content, encoding="utf-8")
    print(f"Stamped module.prop version=v{build_version}")

    # CI sets UPDATE_JSON_URL so Magisk/KSU knows where to check for updates;
    # local dev builds leave it unset and ship without updateJson.
    update_json_url = os.environ.get("UPDATE_JSON_URL")
    if update_json_url:
        with open(module_prop, "a", encoding="utf-8") as f:
            f.write(f"updateJson={update_json_url}\n")

    # Zip it
    out_zip = script_dir / "target" / "vpnhide-zygisk.zip"
    if out_zip.exists():
        out_zip.unlink()

    make_zip(staging, out_zip)

    print()
    print(f"Built: {out_zip.name}")
    size_kb = out_zip.stat().st_size / 1024
    print(f"  {out_zip} ({size_kb:.1f} KB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
