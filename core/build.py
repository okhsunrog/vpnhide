#!/usr/bin/env python3
"""Build the vpnhide-core daemon for Android and package the required
`vpnhide_core` Magisk / KernelSU / APatch module as `vpnhide-core.zip`.

Requirements:
  - rustup targets aarch64-linux-android and armv7-linux-androideabi
  - cargo-ndk
  - Android NDK at $ANDROID_NDK_HOME or auto-detected from $HOME/Android/Sdk/ndk/*

Output: core/target/vpnhide-core.zip. The zip ships one daemon binary per ABI under
`bin/<manager $ARCH>/vpnhide-core`; customize.sh (three unconditional lines) keeps
the one the manager names for the device. All boot logic is the binary's own
`vpnhide-core service`; `service.sh` is a single exec of it.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
from build_lib import (  # type: ignore[import-not-found]
    detect_android_ndk,
    get_build_version,
    make_zip,
)

# Android API level the daemon links against; matches the app's minSdk.
API_LEVEL = "28"

# cargo-ndk ABI -> (rust target triple, the root manager's $ARCH for that ABI)
ABIS = {
    "arm64-v8a": ("aarch64-linux-android", "arm64"),
    "armeabi-v7a": ("armv7-linux-androideabi", "arm"),
}


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parent

    android_ndk_home = detect_android_ndk()
    if not android_ndk_home or not Path(android_ndk_home).is_dir():
        print(
            "error: ANDROID_NDK_HOME not set and no NDK found under ~/Android/Sdk/ndk",
            file=sys.stderr,
        )
        return 1
    print(f"Using NDK: {android_ndk_home}")

    env = os.environ.copy()
    env["ANDROID_NDK_HOME"] = android_ndk_home
    target_dir = Path(env.get("CARGO_TARGET_DIR", repo_root / "target"))

    cmd = ["cargo", "ndk"]
    for abi in ABIS:
        cmd += ["-t", abi]
    # --locked: fail if Cargo.lock drifted rather than rewriting it (a dirtied
    # tree stamps artifacts "-dirty").
    cmd += [
        "-P",
        API_LEVEL,
        "build",
        "--release",
        "--locked",
        "-p",
        "vpnhide_core_daemon",
        "--bin",
        "vpnhide-core",
    ]
    subprocess.run(cmd, cwd=repo_root, env=env, check=True)

    staging = script_dir / "target" / "module-staging"
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(script_dir / "module", staging)
    for abi, (triple, suffix) in ABIS.items():
        built = target_dir / triple / "release" / "vpnhide-core"
        if not built.exists():
            print(f"error: expected {abi} binary at {built}, not found", file=sys.stderr)
            return 1
        dest = staging / "bin" / suffix / "vpnhide-core"
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(built, dest)
        dest.chmod(0o755)

    build_version = get_build_version(repo_root)
    module_prop = staging / "module.prop"
    content = module_prop.read_text(encoding="utf-8")
    content = re.sub(r"^version=.*", f"version=v{build_version}", content, flags=re.MULTILINE)
    module_prop.write_text(content, encoding="utf-8")
    print(f"Stamped module.prop version=v{build_version}")

    # CI sets UPDATE_JSON_URL so the root manager knows where to check for
    # updates; local builds ship without updateJson.
    update_json_url = os.environ.get("UPDATE_JSON_URL")
    if update_json_url:
        with open(module_prop, "a", encoding="utf-8") as f:
            f.write(f"updateJson={update_json_url}\n")

    out_zip = script_dir / "target" / "vpnhide-core.zip"
    if out_zip.exists():
        out_zip.unlink()
    make_zip(staging, out_zip)

    size_kb = out_zip.stat().st_size / 1024
    print()
    print(f"Built: {out_zip} ({size_kb:.1f} KB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
