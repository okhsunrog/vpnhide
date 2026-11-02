#!/usr/bin/env python3
"""Package a pinned fork from locally supplied, hash-bound tools. No downloads."""

import argparse
import hashlib
import json
import stat
import subprocess
import zipfile
from pathlib import Path

MODULE_COMMIT = "b5612adf4d4085a253cb0be30fa1be36e11d09db"
ENGINE_COMMIT = "456744b29efb9989445463ab29e368fa59a103c4"
SAFETY_FILES = {
    "action.sh": "4350f1fd449a1b481a841fc2eec32a3d45d70b29862e7cb485f8200da9aacc92",
    "banner.png": "40f0086d36e73a4f5ee56f90ade4216b218d7327f1304f2c82a212fa255e0c52",
    "customize.sh": "4146cbf29a2fe6a7a53fec97a2eb3af2ba3ced8490563a0c88cda6a2d8260452",
    "module.prop": "bbfe17e5061bae2fbf98b0b6377cf2a1de8f8df20159ef4cb103f0e61d245864",
    "patch/boot_extract.sh": "7aa762c11c6c43c2dc1d51b2cfdb411efcbc06a829612e9b712963079756c6f6",
    "patch/boot_patch.sh": "522af7469fb48fa68af5bcb09f17e86992221c3ebb5fd03217d760506f39b5f3",
    "patch/boot_unpatch.sh": "ea8a4d6d84cfbb46ca0101de3671aefc22577d725631be16644d4a081737c47a",
    "patch/util_functions.sh": "62e4f6d2129c4dee6839bfb2aed80b7b7e01cca900c70daf5a01caa88041584f",
    "post-fs-data.sh": "02a84efa14db03d7acfd9f33957c426f1c4a1e027331e5594c1db076c95aee51",
    "service.sh": "dcd82443f2588cb187536ce0343cd1882f11156d0f21185dae89c662bd595ab1",
    "status.sh": "bb2add0b0beca56c36cf44b83142c5981dee078ee5ca22d0807126c36c5b1571",
    "uninstall.sh": "a9634318d29a87df790a3a8b7ec99c1e9a5d6273bf2d98cefb98b80ba1c2a291",
}
TOOLS = ("kpimg", "kptools", "kpatch", "magiskboot")


def read_regular(path, limit):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or not 0 < info.st_size <= limit:
        raise ValueError(f"not a bounded regular input: {path}")
    data = path.read_bytes()
    after = path.lstat()
    if (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns) != (
        after.st_dev,
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
        after.st_ctime_ns,
    ):
        raise ValueError(f"input changed while reading: {path}")
    return data


def prepare(source, tools, manifest):
    commit = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    if (
        commit != MODULE_COMMIT
        or manifest.get("module_commit") != MODULE_COMMIT
        or manifest.get("engine_commit") != ENGINE_COMMIT
    ):
        raise ValueError("source commits do not match the reviewed fork bases")
    if set(manifest.get("tools", {})) != set(TOOLS):
        raise ValueError("all four pinned tools are required")
    for name, expected in SAFETY_FILES.items():
        if (
            hashlib.sha256(read_regular(source / "module" / name, 1024 * 1024)).hexdigest()
            != expected
        ):
            raise ValueError(f"required reviewed module patch is missing or changed: {name}")
    files = {}
    for name in TOOLS:
        record = manifest["tools"][name]
        data = read_regular(tools / name, 64 * 1024 * 1024)
        if (
            record.get("bytes") != len(data)
            or record.get("sha256") != hashlib.sha256(data).hexdigest()
        ):
            raise ValueError(f"tool hash/size mismatch: {name}")
        origin = record.get("source_commit", "")
        if len(origin) != 40 or any(c not in "0123456789abcdef" for c in origin):
            raise ValueError(f"tool source is not an immutable commit: {name}")
        if name != "magiskboot" and origin != ENGINE_COMMIT:
            raise ValueError(f"engine/tool source tuple mismatch: {name}")
        if name == "kpimg":
            if not data.startswith(b"KP2026"):
                raise ValueError("engine is not the reviewed Next payload family")
        elif (
            len(data) < 64
            or data[:6] != b"\x7fELF\x02\x01"
            or int.from_bytes(data[18:20], "little") != 183
        ):
            raise ValueError(f"tool is not ARM64 little-endian ELF: {name}")
        files[f"bin/{name}"] = (data, 0o755)
    for path in sorted((source / "module").rglob("*")):
        if path.is_dir() and not path.is_symlink():
            continue
        name = path.relative_to(source / "module").as_posix()
        if name.startswith(("bin/", "webroot/")):
            continue
        if name not in SAFETY_FILES:
            raise ValueError(f"unexpected module payload: {name}")
        data = read_regular(path, 4 * 1024 * 1024)
        files[name] = (data, 0o755 if name.endswith(".sh") else 0o644)
    files["provenance.json"] = (
        json.dumps(manifest, sort_keys=True, indent=2).encode() + b"\n",
        0o644,
    )
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(read_regular(args.manifest, 64 * 1024))
    files = prepare(args.source, args.tools, manifest)
    # Exclusive output: a failed build cannot replace an accepted artifact.
    with (
        args.output.open("xb") as output,
        zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as package,
    ):
        for name, (data, mode) in sorted(files.items()):
            entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = (stat.S_IFREG | mode) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            package.writestr(entry, data)
    print(hashlib.sha256(args.output.read_bytes()).hexdigest())


if __name__ == "__main__":
    main()
