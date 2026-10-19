#!/usr/bin/env python3
"""Build the vpnhide KPM and package it as an installable KernelSU/Magisk
module zip — single entry point for CI and local builds.

The `.kpm` is one cross-version relocatable object with reference layouts for
the explicitly supported Android kernel families (4.9, 4.14, 4.19, 5.4, 5.10,
5.15, 6.1, 6.6, and 6.12) on both KernelPatch runtimes, so unlike the `.ko`
there is no per-KMI build matrix: one build, one zip.

Build step: `make -C kmod kpm` (host clang, no kernel tree — only the
KernelPatch submodule headers). Set CLANG_DIR to pick a specific clang;
otherwise `clang` from PATH is used (must bundle lld for the `-r` link).

Output: vpnhide-kpm.zip at the repo root (override with --out).

    ./kmod/kpm/build.py
    CLANG_DIR=/opt/ddk/clang/clang-r*/bin ./kmod/kpm/build.py
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "scripts"))
from build_lib import (  # type: ignore[import-not-found]
    get_build_version,
    make_zip,
)

KPM_FILE = "vpnhide.kpm"


def main() -> int:
    parser = argparse.ArgumentParser(description="Build the vpnhide KPM module zip.")
    parser.add_argument(
        "--out",
        type=Path,
        help="Output zip path. Default: vpnhide-kpm.zip in the repo root.",
    )
    args = parser.parse_args()

    kpm_dir = Path(__file__).resolve().parent
    kmod_dir = kpm_dir.parent
    repo_root = kmod_dir.parent
    build_version = get_build_version(repo_root)

    # Build the .kpm. `make` decides whether anything needs rebuilding; pass
    # CLANG_DIR through if the caller set it (CI / direnv .env).
    make_cmd = ["make", "-C", str(kmod_dir), "kpm"]
    make_cmd.append(f"VPNHIDE_KPM_BUILD_VERSION={build_version}")
    clang_dir = os.environ.get("CLANG_DIR")
    if clang_dir:
        make_cmd.append(f"CLANG_DIR={clang_dir}")
    subprocess.run(make_cmd, check=True)

    kpm_src = kmod_dir / KPM_FILE
    if not kpm_src.exists():
        print(f"error: expected {kpm_src} after `make kpm`, not found", file=sys.stderr)
        return 1

    # Stage the module skeleton, drop the freshly built .kpm in.
    staging = kpm_dir / "module-staging"
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(kpm_dir / "module", staging)
    shutil.copy(kpm_src, staging / KPM_FILE)

    # Stamp the effective build version into the staged module.prop without
    # touching the committed file (keeps PR diffs from churning it).
    module_prop = staging / "module.prop"
    content = module_prop.read_text(encoding="utf-8")
    content = re.sub(r"^version=.*", f"version=v{build_version}", content, flags=re.MULTILINE)
    module_prop.write_text(content, encoding="utf-8")
    print(f"stamped module.prop version=v{build_version}")

    # CI sets UPDATE_JSON_URL so Magisk/KSU knows where to check for updates;
    # local dev builds leave it unset and ship without updateJson.
    update_json_url = os.environ.get("UPDATE_JSON_URL")
    if update_json_url:
        with module_prop.open("a", encoding="utf-8") as f:
            f.write(f"updateJson={update_json_url}\n")

    out_zip = args.out if args.out else repo_root / "vpnhide-kpm.zip"
    if out_zip.exists():
        out_zip.unlink()
    make_zip(staging, out_zip)
    shutil.rmtree(staging)

    size_kb = out_zip.stat().st_size / 1024
    print(f"built {out_zip} ({size_kb:.1f} KB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
