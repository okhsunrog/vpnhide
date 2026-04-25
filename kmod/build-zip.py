#!/usr/bin/env python3
"""Build and package the KernelSU/Magisk kernel module zip.

Assembles a module staging directory so the committed module.prop stays at
its release version while the zip carries the actual build version (git describe).

Usage:
    python3 build-zip.py --kdir /path/to/kdir --kmi android14-5.15  # explicit args
    python3 build-zip.py --kdir /path/to/kdir --kmi android14-5.15 --out custom.zip  # custom output
    # Or use environment variables: KDIR and KMI (CLI args override env vars)
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
from build_lib import get_build_version, make_zip, version_sort_key  # type: ignore[import-not-found]


# Module file names
KMOD_C = "vpnhide_kmod.c"
KMOD_KO = "vpnhide_kmod.ko"


def main() -> int:
    parser = argparse.ArgumentParser(description="Build and package the kernel module zip.")
    parser.add_argument(
        "--kdir",
        type=str,
        help="Kernel source directory (overrides KDIR or KERNEL_SRC env var)",
    )
    parser.add_argument(
        "--kmi",
        type=str,
        help="Kernel module interface variant (e.g., android14-5.15) for module.prop (overrides KMI env var)",
    )
    parser.add_argument(
        "--out",
        type=str,
        help="Output zip filename (default: vpnhide-kmod-<kmi>.zip)",
    )
    parser.add_argument(
        "--clang-dir",
        type=str,
        help="Clang binaries directory (overrides CLANG_DIR env var or auto-detects)",
    )
    args = parser.parse_args()

    kmod_dir = Path(__file__).resolve().parent
    os.chdir(kmod_dir)

    # Resolve kdir: CLI arg > KDIR env var > KERNEL_SRC env var
    if args.kdir:
        kdir = args.kdir
        kdir_src = "--kdir CLI argument"
    else:
        kdir = os.environ.get("KDIR")
        kdir_src = "KDIR env var"
        if not kdir:
            kdir = os.environ.get("KERNEL_SRC")
            kdir_src = "KERNEL_SRC env var"
    if not kdir:
        print("Error: --kdir argument, KDIR, or KERNEL_SRC env var is required", file=sys.stderr)
        return 1

    print(f"Using kdir from {kdir_src}: {kdir}")

    # Resolve kmi: CLI arg > KMI env var
    if args.kmi:
        kmi = args.kmi
        kmi_src = "--kmi CLI argument"
    else:
        kmi = os.environ.get("KMI")
        kmi_src = "KMI env var"
    if not kmi:
        print("Error: --kmi argument or KMI env var is required", file=sys.stderr)
        return 1

    print(f"Using kmi from {kmi_src}: {kmi}")

    # Set up environment for make
    os.environ["KERNEL_SRC"] = kdir

    # Resolve clang_dir: CLI arg > CLANG_DIR env var > auto-detect
    clang_dir = None
    clang_dir_src = None
    if args.clang_dir:
        clang_dir = args.clang_dir
        clang_dir_src = "--clang-dir CLI argument"
    else:
        clang_dir = os.environ.get("CLANG_DIR")
        if clang_dir:
            clang_dir_src = "CLANG_DIR env var"
        else:
            # Auto-detect: In CI, clang is at /opt/ddk/clang/clang-r*/bin
            clang_base = Path("/opt/ddk/clang")
            if clang_base.exists():
                clang_dirs = sorted(
                    (d for d in clang_base.iterdir() if d.is_dir() and d.name.startswith("clang-")),
                    key=lambda p: version_sort_key(p.name),
                )
                if clang_dirs:
                    clang_dir = str(clang_dirs[-1] / "bin")
                    clang_dir_src = "auto-detected from /opt/ddk/clang"
    if clang_dir:
        os.environ["CLANG_DIR"] = clang_dir
        print(f"Using clang-dir from {clang_dir_src}: {clang_dir}")
    else:
        print("Warning: clang-dir not set, using system PATH", file=sys.stderr)

    # Build the kernel module — let make decide whether anything needs
    # rebuilding; its dependency tracking covers all sources, headers,
    # and the kernel .config, not just vpnhide_kmod.c.
    print("Building kernel module...")
    subprocess.run(["make", "strip"], check=True)

    # Assemble the module staging directory so the committed module.prop
    # stays at its release version while the zip carries the actual build
    # version (git describe).
    staging = kmod_dir / "module-staging"
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(kmod_dir / "module", staging)
    shutil.copy(kmod_dir / KMOD_KO, staging / KMOD_KO)

    # Get build version
    build_version = get_build_version(kmod_dir.parent)

    # Stamp version into module.prop
    module_prop = staging / "module.prop"
    content = module_prop.read_text(encoding="utf-8")
    content = re.sub(r"^version=.*", f"version=v{build_version}", content, flags=re.MULTILINE)
    # Add gkiVariant and updateJson
    content = re.sub(r"^gkiVariant=.*", f"gkiVariant={kmi}", content, flags=re.MULTILINE)
    if not re.search(r"^gkiVariant=", content, flags=re.MULTILINE):
        content = content.rstrip() + f"\ngkiVariant={kmi}\n"
    update_json_url = f"https://raw.githubusercontent.com/okhsunrog/vpnhide/main/update-json/update-kmod-{kmi}.json"
    content = re.sub(r"^updateJson=.*", f"updateJson={update_json_url}", content, flags=re.MULTILINE)
    if not re.search(r"^updateJson=", content, flags=re.MULTILINE):
        content = content.rstrip() + f"\nupdateJson={update_json_url}\n"
    module_prop.write_text(content, encoding="utf-8")
    print(f"Stamped module.prop version=v{build_version} gkiVariant={kmi}")

    # Create zip in parent directory (workspace root for CI)
    out_zip = kmod_dir.parent / (args.out if args.out else f"vpnhide-kmod-{kmi}.zip")
    if out_zip.exists():
        out_zip.unlink()

    make_zip(staging, out_zip)
    shutil.rmtree(staging)

    print()
    print(f"Built: {out_zip.name}")
    size_kb = out_zip.stat().st_size / 1024
    print(f"  {out_zip} ({size_kb:.1f} KB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
