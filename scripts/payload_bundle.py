"""Build provenance and bounded, passive unified Core assembly (stdlib only).

This verifies consistency within one trusted CI run, not publisher signatures.
It never extracts paths chosen by an input ZIP or enables Zygisk autoload.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import tempfile
import zipfile
from pathlib import Path

from kmod_elf import inspect as inspect_kmod

KMIS = (
    "android12-5.10",
    "android13-5.10",
    "android13-5.15",
    "android14-5.15",
    "android14-6.1",
    "android15-6.6",
    "android16-6.12",
)
MAX_FILE = 128 * 1024 * 1024
MAX_TOTAL = 256 * 1024 * 1024
PROVENANCE = "build-provenance.json"


def stream_digest(file) -> str:
    # DDK producer images may ship Python 3.10 (no hashlib.file_digest).
    checksum = hashlib.sha256()
    while block := file.read(64 * 1024):
        checksum.update(block)
    return checksum.hexdigest()


def digest(path: Path) -> str:
    with path.open("rb") as file:
        return stream_digest(file)


def stamp(staging: Path, repo: Path, component: str, version: str) -> None:
    """Attach actual checkout identity and exact staged-file hashes before zipping."""
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    dirty = bool(
        subprocess.check_output(
            ["git", "status", "--porcelain", "--untracked-files=no"],
            cwd=repo,
            text=True,
        ).strip()
    )
    files = {}
    for path in sorted(staging.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"symlink in staging: {path}")
        if path.is_file() and path.name != PROVENANCE:
            files[path.relative_to(staging).as_posix()] = {
                "bytes": path.stat().st_size,
                "sha256": digest(path),
            }
    (staging / PROVENANCE).write_text(
        json.dumps(
            {
                "schema": 1,
                "source_commit": commit,
                "dirty": dirty,
                "component": component,
                "version": version,
                "files": files,
            },
            sort_keys=True,
        )
        + "\n"
    )


class Artifact:
    def __init__(self, path: Path, component: str, commit: str):
        self.zip = zipfile.ZipFile(path)
        try:
            infos = self.zip.infolist()
            names = [info.filename for info in infos]
            if len(names) != len(set(names)) or len(names) > 64:
                raise ValueError("duplicate or excessive archive members")
            total = 0
            for info in infos:
                parts = info.filename.split("/")
                if (
                    any(p in ("", ".", "..") for p in parts)
                    or "\\" in info.filename
                    or stat.S_ISLNK(info.external_attr >> 16)
                    or info.file_size > MAX_FILE
                    or info.flag_bits & 1
                ):
                    raise ValueError("unsafe archive member")
                total += info.file_size
            if total > MAX_TOTAL or self.zip.getinfo(PROVENANCE).file_size > 64 * 1024:
                raise ValueError("artifact exceeds budget")
            self.meta = json.loads(self.zip.read(PROVENANCE))
            if (
                set(self.meta)
                != {"schema", "source_commit", "dirty", "component", "version", "files"}
                or self.meta["schema"] != 1
                or self.meta["dirty"] is not False
                or self.meta["source_commit"] != commit
                or self.meta["component"] != component
                or not isinstance(self.meta["version"], str)
                or not 0 < len(self.meta["version"]) <= 128
                or any(ord(c) < 32 for c in self.meta["version"])
            ):
                raise ValueError("artifact provenance mismatch or dirty build")
            if set(self.meta["files"]) != set(names) - {PROVENANCE}:
                raise ValueError("provenance must cover every artifact member")
            for name, expected in self.meta["files"].items():
                if set(expected) != {"bytes", "sha256"}:
                    raise ValueError("invalid file provenance")
                with self.zip.open(name) as file:
                    actual = stream_digest(file)
                if expected != {"bytes": self.zip.getinfo(name).file_size, "sha256": actual}:
                    raise ValueError("artifact hash/size mismatch")
        except BaseException:
            self.zip.close()
            raise

    def copy(self, name: str, target: Path, machine: int | None = None) -> None:
        """Copy an explicitly named member; no archive-directed extraction."""
        if machine is not None:
            with self.zip.open(name) as file:
                header = file.read(20)
            elf_class = 2 if machine == 183 else 1
            if (
                len(header) != 20
                or header[:6] != b"\x7fELF" + bytes([elf_class, 1])
                or int.from_bytes(header[18:20], "little") != machine
            ):
                raise ValueError(f"wrong ELF architecture: {name}")
        target.parent.mkdir(parents=True, exist_ok=True)
        with self.zip.open(name) as source, target.open("wb") as dest:
            while block := source.read(64 * 1024):
                dest.write(block)
        target.chmod(
            0o755
            if name.startswith("bin/") or name.endswith(".sh") or name.endswith("update-binary")
            else 0o644
        )


def assemble(inputs: Path, output: Path, commit: str) -> None:
    if not re.fullmatch("[0-9a-f]{40}", commit):
        raise ValueError("expected exact source commit")
    specs = [("core", "vpnhide-core-base.zip")]
    specs += [(f"kmod:{kmi}", f"vpnhide-kmod-{kmi}.zip") for kmi in KMIS]
    specs += [("kpm", "vpnhide-kpm.zip"), ("zygisk", "vpnhide-zygisk.zip")]
    artifacts = {}
    output.parent.mkdir(parents=True, exist_ok=True)
    try:
        for component, name in specs:
            artifacts[component] = Artifact(inputs / name, component, commit)
        with tempfile.TemporaryDirectory(prefix="core-assembly-", dir=output.parent) as temporary:
            staging = Path(temporary) / "module"
            core = artifacts["core"]
            if b"id=vpnhide_core" not in core.zip.read("module.prop").splitlines():
                raise ValueError("unexpected Core module identity")
            for name, machine in [("bin/arm64/vpnhide-core", 183), ("bin/arm/vpnhide-core", 40)]:
                core.copy(name, staging / name, machine)
            for name in core.meta["files"]:
                if name.startswith(("zygisk/", "payloads/")) or name == "payloads/manifest.json":
                    raise ValueError("base Core contains unexpected payloads")
                core.copy(name, staging / name)
            payloads = []
            for kmi in KMIS:
                payloads.append(
                    (
                        f"kmod:{kmi}",
                        "vpnhide_kmod.ko",
                        f"payloads/kmod/{kmi}/vpnhide_kmod.ko",
                        {"kind": "kmod", "kmi": kmi},
                        183,
                    )
                )
            payloads.append(
                ("kpm", "vpnhide.kpm", "payloads/kpm/arm64/vpnhide.kpm", {"kind": "kpm"}, 183)
            )
            for abi, architecture, machine in [
                ("arm64-v8a", "arm64", 183),
                ("armeabi-v7a", "arm32", 40),
            ]:
                payloads.append(
                    (
                        "zygisk",
                        f"zygisk/{abi}.so",
                        f"payloads/zygisk/{abi}.so",
                        {"kind": "zygisk", "abi": architecture},
                        machine,
                    )
                )
            manifest = {"schema": 1, "bundle_commit": commit, "payloads": []}
            total = 0
            for component, source, target, implementation, machine in payloads:
                artifact = artifacts[component]
                artifact.copy(source, staging / target, machine)
                proof = artifact.meta["files"][source]
                capability = {}
                if implementation["kind"] == "kmod":
                    # Inspect exact final copied bytes, never producer version text.
                    protocol = inspect_kmod((staging / target).read_bytes())["preflight_protocol"]
                    if protocol != 1:
                        raise ValueError("unified kmod lacks verified preflight protocol 1")
                    capability["preflight_protocol"] = protocol
                if not 0 < proof["bytes"] <= MAX_FILE:
                    raise ValueError("payload exceeds budget")
                total += proof["bytes"]
                manifest["payloads"].append(
                    {
                        "implementation": implementation,
                        "version": artifact.meta["version"],
                        "source_commit": commit,
                        **proof,
                        "control_version": 2,
                        **capability,
                    }
                )
            if total > MAX_TOTAL:
                raise ValueError("bundle exceeds budget")
            (staging / "payloads/manifest.json").write_text(
                json.dumps(manifest, sort_keys=True) + "\n"
            )
            # Keep exact source proofs for diagnosis. No bootstrap scripts from backends.
            (staging / "bundle-provenance.json").write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "source_commit": commit,
                        "inputs": {name: artifact.meta for name, artifact in artifacts.items()},
                    },
                    sort_keys=True,
                )
                + "\n"
            )
            if (
                sum(path.stat().st_size for path in staging.rglob("*") if path.is_file())
                > MAX_TOTAL
            ):
                raise ValueError("assembled archive exceeds budget")
            staged_zip = Path(temporary) / "bundle.zip"
            with zipfile.ZipFile(staged_zip, "w", zipfile.ZIP_DEFLATED) as zipfile_out:
                for path in sorted(staging.rglob("*")):
                    if path.is_file():
                        zipfile_out.write(path, path.relative_to(staging).as_posix())
            os.replace(staged_zip, output)
    finally:
        for artifact in artifacts.values():
            artifact.zip.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    assemble(args.inputs, args.output, args.commit)
