#!/usr/bin/env python3
"""Package the built-in backend's companion module into a flashable zip.

Unlike kmod/build.py this compiles no kernel object and is KMI-agnostic: the
driver is baked into the user's kernel (CONFIG_VPNHIDE=y, via builtin/scripts/
apply.sh), so the zip ships only the userspace glue — the Rust `builtin`
activator plus the boot scripts in builtin/module/. One zip works on every
kernel that has the driver compiled in.

    builtin/build.py [--out PATH]

Output: vpnhide-builtin.zip (repo root by default).
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
from pathlib import Path

BUILTIN_DIR = Path(__file__).resolve().parent
REPO_ROOT = BUILTIN_DIR.parent
sys.path.insert(0, str(REPO_ROOT / "scripts"))

from build_lib import (  # type: ignore[import-not-found]  # noqa: E402
    build_activator_bin,
    get_build_version,
    make_zip,
)

UPDATE_JSON_URL = (
    "https://raw.githubusercontent.com/okhsunrog/vpnhide/main/update-json/update-builtin.json"
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, help="output zip path (default: repo/vpnhide-builtin.zip)")
    args = ap.parse_args()

    staging = BUILTIN_DIR / "module-staging"
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(BUILTIN_DIR / "module", staging)

    # Cross-compile the `builtin` activator bin for Android arm64 and drop it in
    # as $MODDIR/activator (the path the boot scripts exec).
    activator = build_activator_bin(REPO_ROOT, "builtin")
    assert activator is not None  # required=True by default: raises if unavailable
    shutil.copy(activator, staging / "activator")
    (staging / "activator").chmod(0o755)

    # Stamp the build version + update URL into module.prop. The committed
    # module.prop keeps the last release version so PR diffs don't churn it; the
    # gkiVariant field the .ko carries is intentionally absent — this module is
    # KMI-agnostic. versionCode is left to the release flow (as in kmod).
    build_version = get_build_version(REPO_ROOT)
    module_prop = staging / "module.prop"
    content = module_prop.read_text(encoding="utf-8")
    content = re.sub(r"^version=.*", f"version=v{build_version}", content, flags=re.MULTILINE)
    if re.search(r"^updateJson=", content, flags=re.MULTILINE):
        content = re.sub(
            r"^updateJson=.*", f"updateJson={UPDATE_JSON_URL}", content, flags=re.MULTILINE
        )
    else:
        content = content.rstrip() + f"\nupdateJson={UPDATE_JSON_URL}\n"
    module_prop.write_text(content, encoding="utf-8")
    print(f"[builtin] stamped module.prop version=v{build_version}")

    out_zip = args.out or REPO_ROOT / "vpnhide-builtin.zip"
    if out_zip.exists():
        out_zip.unlink()
    make_zip(staging, out_zip)
    shutil.rmtree(staging)

    size_kb = out_zip.stat().st_size / 1024
    print(f"[builtin] built {out_zip} ({size_kb:.1f} KB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
