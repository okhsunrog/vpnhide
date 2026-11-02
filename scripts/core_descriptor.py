"""Sign the final unified ZIP with the existing Android key, never a new key.

The exact descriptor bytes are signed with a fixed domain prefix. Public trust
is exported before Core compilation; signed package identity is embedded in the
APK afterwards. No release, upload or network request is performed here.
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

from payload_bundle import KMIS

MAX_PACKAGE = 256 * 1024 * 1024
SIGNER = Path(__file__).with_name("CoreDescriptorSigner.java")
REPO = Path(__file__).resolve().parent.parent


def member_identity(archive: zipfile.ZipFile, name: str, limit: int) -> dict:
    size = archive.getinfo(name).file_size
    if not 0 < size <= limit:
        raise ValueError("runtime member exceeds limit")
    digest = hashlib.sha256()
    count = 0
    with archive.open(name) as file:
        while block := file.read(64 * 1024):
            count += len(block)
            if count > size:
                raise ValueError("runtime member exceeds declared size")
            digest.update(block)
    if count != size:
        raise ValueError("incomplete runtime member")
    return {"bytes": size, "sha256": digest.hexdigest()}


def invoke(mode: str, keystore: Path, source: Path, destination: Path) -> None:
    result = subprocess.run(
        ["java", str(SIGNER), mode, str(keystore), str(source), str(destination)],
        capture_output=True,
        check=False,
    )
    if result.returncode:
        # Provider output can contain aliases and sensitive paths.
        raise ValueError("existing Android signing identity is unavailable or unsupported")


def integer_constant(path: Path, name: str) -> int:
    matches = re.findall(rf"^\s*(?:pub )?const {name}: u32 = (\d+);$", path.read_text(), re.M)
    if len(matches) != 1:
        raise ValueError(f"missing or ambiguous compatibility constant: {name}")
    return int(matches[0])


def descriptor(package: Path, commit: str, sequence: int) -> dict:
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or not 0 < sequence <= 2**64 - 1:
        raise ValueError("invalid source or build sequence")
    size = package.stat().st_size
    if not 0 < size <= MAX_PACKAGE:
        raise ValueError("package exceeds limit")
    with zipfile.ZipFile(package) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or len(names) > 128:
            raise ValueError("duplicate or excessive archive entries")
        total = 0
        for info in archive.infolist():
            if (
                any(part in ("", ".", "..") for part in info.filename.split("/"))
                or "\\" in info.filename
                or stat.S_ISLNK(info.external_attr >> 16)
                or info.flag_bits & 1
                or info.file_size > 128 * 1024 * 1024
            ):
                raise ValueError("unsafe package member")
            total += info.file_size
            if total > MAX_PACKAGE:
                raise ValueError("expanded package exceeds limit")
        for name in ("bundle-provenance.json", "payloads/manifest.json", "module.prop"):
            if archive.getinfo(name).file_size > 64 * 1024:
                raise ValueError("package metadata exceeds limit")
        provenance = json.loads(archive.read("bundle-provenance.json"))
        manifest = json.loads(archive.read("payloads/manifest.json"))
        if provenance.get("source_commit") != commit or manifest.get("bundle_commit") != commit:
            raise ValueError("not a same-commit unified Core package")
        if b"id=vpnhide_core" not in archive.read("module.prop").splitlines():
            raise ValueError("wrong module identity")
        for component in provenance.get("inputs", {}).values():
            if component.get("dirty") is not False or component.get("source_commit") != commit:
                raise ValueError("dirty or mixed package input")
        if not provenance.get("inputs") or len(manifest.get("payloads", [])) != 10:
            # Seven ko, one KPM and both Zygisk ABIs. The assembly tool is the
            # producer; a daemon-only ZIP cannot become an installable package.
            raise ValueError("incomplete unified payload set")
        expected = [
            (dict(kind="kmod", kmi=kmi), f"payloads/kmod/{kmi}/vpnhide_kmod.ko") for kmi in KMIS
        ]
        expected += [(dict(kind="kpm"), "payloads/kpm/arm64/vpnhide.kpm")]
        expected += [
            (dict(kind="zygisk", abi=arch), f"payloads/zygisk/{abi}.so")
            for arch, abi in (("arm64", "arm64-v8a"), ("arm32", "armeabi-v7a"))
        ]
        remaining = list(manifest["payloads"])
        for implementation, path in expected:
            matches = [p for p in remaining if p.get("implementation") == implementation]
            if len(matches) != 1:
                raise ValueError("missing or duplicate payload identity")
            payload = matches[0]
            remaining.remove(payload)
            actual = member_identity(archive, path, 128 * 1024 * 1024)
            if (
                payload.get("source_commit") != commit
                or payload.get("control_version") != 2
                or any(payload.get(k) != v for k, v in actual.items())
            ):
                raise ValueError("payload provenance or bytes differ")
        runtime = {
            "daemon_arm64": member_identity(archive, "bin/arm64/vpnhide-core", 128 * 1024 * 1024),
            "daemon_arm": member_identity(archive, "bin/arm/vpnhide-core", 128 * 1024 * 1024),
            "payload_manifest": member_identity(archive, "payloads/manifest.json", 64 * 1024),
        }
    with package.open("rb") as file:
        digest = hashlib.sha256()
        while block := file.read(64 * 1024):
            digest.update(block)
    return {
        "schema": 1,
        "module_id": "vpnhide_core",
        "source_commit": commit,
        "build_sequence": sequence,
        "package_sha256": digest.hexdigest(),
        "package_bytes": size,
        "rpc": integer_constant(REPO / "crates/core-rpc/src/lib.rs", "PROTOCOL_VERSION"),
        "journal_min": 1,
        "journal_max": integer_constant(
            REPO / "crates/core-daemon/src/journal.rs", "JOURNAL_VERSION"
        ),
        "kernel_control": 2,
        "runtime": runtime,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("public", "sign"))
    parser.add_argument("--keystore", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--commit")
    parser.add_argument("--sequence", type=int)
    parser.add_argument("--trust", type=Path)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(dir=args.output.parent) as temporary:
            source = Path(temporary) / "descriptor.json"
            output = Path(temporary) / "output"
            if args.mode == "public":
                invoke("public", args.keystore, source, output)
                result = output.read_bytes()
            else:
                if any(v is None for v in (args.package, args.commit, args.sequence, args.trust)):
                    raise ValueError(
                        "package, commit, sequence and compiled public trust are required"
                    )
                public = Path(temporary) / "public.json"
                invoke("public", args.keystore, source, public)
                if json.loads(public.read_bytes()) != json.loads(args.trust.read_bytes()):
                    raise ValueError("signer differs from Core's compiled trust identity")
                raw = json.dumps(
                    descriptor(args.package, args.commit, args.sequence),
                    sort_keys=True,
                    separators=(",", ":"),
                )
                source.write_text(raw, encoding="utf-8")
                invoke("sign", args.keystore, source, output)
                result = (
                    json.dumps({"descriptor": raw, "signature": output.read_bytes().hex()}) + "\n"
                ).encode()
            staged = Path(temporary) / "final"
            staged.write_bytes(result)
            os.replace(staged, args.output)
    except (ValueError, OSError, KeyError, zipfile.BadZipFile, json.JSONDecodeError):
        parser.exit(1, "Core package signing failed; no fallback key or descriptor is generated.\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
