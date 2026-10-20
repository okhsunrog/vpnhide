#!/usr/bin/env python3
"""Build the vpnhide kernel module zip — single entry point for both CI
and local builds.

Two modes, picked automatically:

 1. **Native build** — this Python process compiles `vpnhide_kmod.ko` and
    packages the zip directly. Used when:

    - `--inside-container` is passed (CI does this for clarity), OR
    - `--kdir` is passed / `KDIR`|`KERNEL_SRC` is in env (local kernel
      source build via direnv), OR
    - `/opt/ddk/clang` is present (we're already inside the
      `ghcr.io/ylarod/ddk-min` image — auto-detects kdir + clang under
      `/opt/ddk`).

 2. **Container build** — this Python process spawns docker/podman,
    bind-mounts the repo, and re-invokes itself with
    `--inside-container`. Used when none of the native conditions apply,
    i.e. the typical local `./kmod/build.py --kmi android14-6.1`
    workflow on a developer machine.

Either way the output is `vpnhide-kmod-<kmi>.zip` at the repo root,
identical between CI and local.

Examples:
    ./kmod/build.py --kmi android14-6.1            # local, default
    ./kmod/build.py --all                          # every GKI variant
    ./kmod/build.py --kdir ~/k/android14-6.1 --kmi android14-6.1
                                                    # local kernel source

The DDK container tag (`DDK_IMAGE_TAG`) is the single source of truth for
both this script and `.github/workflows/ci.yml`'s kmod matrix — keep them
in sync when bumping.
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
from build_lib import (  # type: ignore[import-not-found]
    get_build_version,
    make_zip,
    version_sort_key,
)

# Module file name on disk after `make`.
KMOD_KO = "vpnhide_kmod.ko"

# Tag of `ghcr.io/ylarod/ddk-min:<kmi>-<TAG>`. Keep this in lockstep with
# the same constant in `.github/workflows/ci.yml` so a bump rebuilds
# locally and in CI from the exact same image.
DDK_IMAGE_TAG = "20260313"

# Every GKI variant we publish a kmod for. Order matches the CI matrix.
GKI_VARIANTS = (
    "android12-5.10",
    "android13-5.10",
    "android13-5.15",
    "android14-5.15",
    "android14-6.1",
    "android15-6.6",
    "android16-6.12",
)

DEFAULT_KMI = "android14-6.1"


def detect_clang_dir() -> str | None:
    """Pick the highest-versioned `clang-r*/bin` under `/opt/ddk/clang`,
    matching what the DDK image lays out. Used only when the user
    didn't pass --clang-dir or set CLANG_DIR."""
    clang_base = Path("/opt/ddk/clang")
    if not clang_base.is_dir():
        return None
    candidates = sorted(
        (d for d in clang_base.iterdir() if d.is_dir() and d.name.startswith("clang-")),
        key=lambda p: version_sort_key(p.name),
    )
    return str(candidates[-1] / "bin") if candidates else None


def detect_kdir(kmi: str) -> str | None:
    """`/opt/ddk/kdir/<kmi>` is laid out by the DDK image."""
    kdir = Path("/opt/ddk/kdir") / kmi
    return str(kdir) if kdir.is_dir() else None


def native_build_one(
    kmod_dir: Path,
    kmi: str,
    kdir: str,
    clang_dir: str | None,
    out: Path | None,
) -> int:
    """Compile + package one .ko into one zip, in the current process."""
    print(f"[{kmi}] kdir={kdir}")
    print(f"[{kmi}] clang-dir={clang_dir or '(system PATH)'}")

    env = os.environ.copy()
    env["KERNEL_SRC"] = kdir
    if clang_dir:
        env["CLANG_DIR"] = clang_dir

    # `make strip` does the actual kernel-module build. Let make decide
    # whether anything needs rebuilding — its dependency tracking covers
    # all sources, headers, and the kernel .config, not just our .c file.
    subprocess.run(["make", "-C", str(kmod_dir), "strip"], env=env, check=True)

    # Stage the module skeleton from kmod/module/, drop the freshly built
    # .ko in, patch module.prop with the real build version + gkiVariant
    # + updateJson. The committed module.prop stays at the last release
    # version so PR diffs don't churn it.
    staging = kmod_dir / "module-staging"
    if staging.exists():
        shutil.rmtree(staging)
    shutil.copytree(kmod_dir / "module", staging)
    shutil.copy(kmod_dir / KMOD_KO, staging / KMOD_KO)

    build_version = get_build_version(kmod_dir.parent)

    module_prop = staging / "module.prop"
    content = module_prop.read_text(encoding="utf-8")
    content = re.sub(r"^version=.*", f"version=v{build_version}", content, flags=re.MULTILINE)
    if re.search(r"^gkiVariant=", content, flags=re.MULTILINE):
        content = re.sub(r"^gkiVariant=.*", f"gkiVariant={kmi}", content, flags=re.MULTILINE)
    else:
        content = content.rstrip() + f"\ngkiVariant={kmi}\n"
    update_json_url = (
        f"https://raw.githubusercontent.com/okhsunrog/vpnhide/main/update-json/"
        f"update-kmod-{kmi}.json"
    )
    if re.search(r"^updateJson=", content, flags=re.MULTILINE):
        content = re.sub(
            r"^updateJson=.*", f"updateJson={update_json_url}", content, flags=re.MULTILINE
        )
    else:
        content = content.rstrip() + f"\nupdateJson={update_json_url}\n"
    module_prop.write_text(content, encoding="utf-8")
    print(f"[{kmi}] stamped module.prop version=v{build_version} gkiVariant={kmi}")

    out_zip = out if out else kmod_dir.parent / f"vpnhide-kmod-{kmi}.zip"
    if out_zip.exists():
        out_zip.unlink()
    make_zip(staging, out_zip)
    shutil.rmtree(staging)

    size_kb = out_zip.stat().st_size / 1024
    print(f"[{kmi}] built {out_zip} ({size_kb:.1f} KB)")
    return 0


def run_native_mode(args: argparse.Namespace, kmod_dir: Path) -> int:
    """Resolve kdir + clang-dir from args/env/auto-detect and build each
    requested kmi natively. Multi-kmi only makes sense when kdir is the
    DDK layout `/opt/ddk/kdir/<kmi>` (auto-detected per kmi)."""
    kmis = _select_kmis(args)

    explicit_kdir = args.kdir or os.environ.get("KDIR") or os.environ.get("KERNEL_SRC")
    explicit_clang = args.clang_dir or os.environ.get("CLANG_DIR")

    if explicit_kdir and len(kmis) > 1:
        print(
            "error: --kdir / KDIR / KERNEL_SRC selects exactly one kernel tree, "
            "so building multiple --kmi values doesn't make sense. "
            "Drop --kdir to use auto-detection from /opt/ddk/kdir/<kmi>.",
            file=sys.stderr,
        )
        return 2
    if args.out and len(kmis) > 1:
        print("error: --out is only valid with a single --kmi", file=sys.stderr)
        return 2

    for kmi in kmis:
        kdir = explicit_kdir or detect_kdir(kmi)
        if not kdir:
            print(
                f"error[{kmi}]: no kernel source. Pass --kdir, set KDIR/KERNEL_SRC, "
                f"or run inside ghcr.io/ylarod/ddk-min where /opt/ddk/kdir/{kmi} exists.",
                file=sys.stderr,
            )
            return 1
        clang_dir = explicit_clang or detect_clang_dir()
        rc = native_build_one(kmod_dir, kmi, kdir, clang_dir, args.out)
        if rc:
            return rc
    return 0


# ----- Container orchestration --------------------------------------------


def find_runtime() -> tuple[str, bool]:
    """(binary, is_podman). Prefer Docker when both are present.

    The CI and device-test workflows use Docker, so matching that locally
    avoids rootless podman bind-mount/userns surprises. Podman remains a
    supported fallback for hosts without Docker.
    """
    docker = shutil.which("docker")
    if docker:
        return docker, False
    podman = shutil.which("podman")
    if podman:
        return podman, True
    print(
        "error: neither podman nor docker found in PATH. Install one, or "
        "build natively by passing --kdir <kernel source> + --inside-container.",
        file=sys.stderr,
    )
    sys.exit(1)


def container_build_one(runtime: str, is_podman: bool, repo_root: Path, kmi: str) -> None:
    image = f"ghcr.io/ylarod/ddk-min:{kmi}-{DDK_IMAGE_TAG}"
    mount_spec = f"{repo_root}:/work"
    build_version = get_build_version(repo_root)
    cmd = [runtime, "run", "--rm"]
    if is_podman:
        # Rootless podman + Fedora SELinux: keep host UID inside so the
        # mount stays writable; ":Z" relabels the bind source so the
        # container can read/write it.
        cmd += ["--userns=keep-id"]
        mount_spec += ":Z"
    cmd += [
        "-e",
        f"VPNHIDE_BUILD_VERSION={build_version}",
        "-v",
        mount_spec,
        "-w",
        "/work",
        image,
        "python3",
        "kmod/build.py",
        "--inside-container",
        "--kmi",
        kmi,
    ]
    print(f"[{kmi}] {' '.join(cmd)}", flush=True)
    subprocess.run(cmd, check=True)


def run_container_mode(args: argparse.Namespace, repo_root: Path) -> int:
    if args.kdir or args.clang_dir or args.out:
        # These flags only make sense in native mode — refusing here is
        # better than silently dropping them after a 2-minute container
        # spin-up.
        print(
            "error: --kdir / --clang-dir / --out are only valid with native "
            "builds (pass --inside-container or run inside the DDK image).",
            file=sys.stderr,
        )
        return 2

    kmis = _select_kmis(args)
    runtime, is_podman = find_runtime()
    print(f"Using {'podman' if is_podman else 'docker'} at {runtime}")
    for kmi in kmis:
        container_build_one(runtime, is_podman, repo_root, kmi)

    print()
    print("Built artifacts (at repo root):")
    for kmi in kmis:
        out = repo_root / f"vpnhide-kmod-{kmi}.zip"
        marker = "ok" if out.is_file() else "MISSING"
        size = f"{out.stat().st_size / 1024:.1f} KB" if out.is_file() else ""
        print(f"  [{marker}] {out.name} {size}")
    return 0


# ----- Argument parsing ---------------------------------------------------


def _select_kmis(args: argparse.Namespace) -> tuple[str, ...]:
    if args.all:
        return GKI_VARIANTS
    if args.kmi:
        return tuple(args.kmi)
    return (DEFAULT_KMI,)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build the vpnhide kernel module zip (CI + local).",
    )
    parser.add_argument(
        "--kmi",
        action="append",
        choices=GKI_VARIANTS,
        help=f"GKI variant to build (repeatable). Default: {DEFAULT_KMI}.",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Build every supported GKI variant (same matrix as CI).",
    )
    parser.add_argument(
        "--inside-container",
        action="store_true",
        help=(
            "Force native build in the current process. CI passes this "
            "explicitly; locally you only need it if you're inside a "
            "container (or have a kernel source set up) and the auto-"
            "detect didn't pick that up."
        ),
    )
    parser.add_argument(
        "--kdir",
        type=str,
        help=("Kernel source directory (overrides KDIR/KERNEL_SRC). Implies native mode."),
    )
    parser.add_argument(
        "--clang-dir",
        type=str,
        help=(
            "Clang binaries directory (overrides CLANG_DIR; auto-detected "
            "from /opt/ddk/clang/clang-r* in DDK images)."
        ),
    )
    parser.add_argument(
        "--out",
        type=Path,
        help="Output zip path (single --kmi only). Default: vpnhide-kmod-<kmi>.zip in repo root.",
    )
    args = parser.parse_args()

    if args.all and args.kmi:
        parser.error("--all and --kmi are mutually exclusive")

    kmod_dir = Path(__file__).resolve().parent
    repo_root = kmod_dir.parent

    # Native conditions: explicit flag, explicit kernel source, or we're
    # already in a DDK image.
    native = (
        args.inside_container
        or bool(args.kdir)
        or "KDIR" in os.environ
        or "KERNEL_SRC" in os.environ
        or detect_clang_dir() is not None
    )
    if native:
        return run_native_mode(args, kmod_dir)
    return run_container_mode(args, repo_root)


if __name__ == "__main__":
    sys.exit(main())
