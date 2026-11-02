"""Bounded inspection of final ARM64 .ko files; never loads a module.

CRC evidence describes imported exported symbols, not private-structure ABI.
The report intentionally preserves ambiguity when adjacent KMIs look alike.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path

MAX_BYTES = 128 * 1024 * 1024


def region(data: bytes, offset: int, size: int) -> bytes:
    if offset < 0 or size < 0 or offset > len(data) or size > len(data) - offset:
        raise ValueError("ELF section outside file")
    return data[offset : offset + size]


def string(data: bytes, offset: int, limit: int = 1024) -> str:
    if offset < 0 or offset >= len(data):
        raise ValueError("ELF string outside table")
    end = data.find(b"\0", offset, offset + limit + 1)
    if end < 0:
        raise ValueError("unterminated ELF string")
    return data[offset:end].decode("ascii")


def inspect(data: bytes) -> dict:
    if (
        len(data) < 64
        or len(data) > MAX_BYTES
        or data[:7] != b"\x7fELF\x02\x01\x01"
        or struct.unpack_from("<HHI", data, 16) != (1, 183, 1)
    ):
        raise ValueError("expected bounded ELF64 little-endian AArch64 relocatable")
    offset = struct.unpack_from("<Q", data, 40)[0]
    entry_size, count, names_index = struct.unpack_from("<HHH", data, 58)
    if entry_size != 64 or not 0 < count <= 4096 or not 0 < names_index < count:
        raise ValueError("unsupported section table")
    table = region(data, offset, count * entry_size)
    headers = [struct.unpack_from("<IIQQQQIIQQ", table, i * 64) for i in range(count)]
    names_header = headers[names_index]
    if names_header[1] != 3 or names_header[5] > 1024 * 1024:
        raise ValueError("invalid section-name table")
    names = region(data, names_header[4], names_header[5])
    sections = {}
    for header in headers[1:]:
        name = string(names, header[0], 256)
        if name in sections:
            raise ValueError("duplicate ELF section")
        sections[name] = header
        if header[1] != 8:  # SHT_NOBITS occupies no bytes in the file.
            region(data, header[4], header[5])

    def body(name: str) -> bytes:
        header = sections.get(name)
        if header is None:
            return b""
        if header[1] == 8:
            raise ValueError("metadata cannot use SHT_NOBITS")
        return region(data, header[4], header[5])

    modinfo = body(".modinfo")
    if ".modinfo" in sections and (sections[".modinfo"][1] != 1 or len(modinfo) > 64 * 1024):
        raise ValueError("invalid module metadata section")
    if modinfo and not modinfo.endswith(b"\0"):
        raise ValueError("unterminated module metadata")
    records = [record.decode("ascii") for record in modinfo.split(b"\0") if record]
    protocol = [
        record.split("=", 1)[1] for record in records if record.startswith("vpnhide_preflight=")
    ]
    parameter = [record for record in records if record.startswith("parmtype=preflight_only:")]
    constructors = any(body(name) for name in (".init_array", ".ctors", ".preinit_array"))
    capability = None
    if protocol:
        if protocol != ["1"] or parameter != ["parmtype=preflight_only:bool"] or constructors:
            raise ValueError("unsupported or inconsistent preflight capability")
        capability = 1

    imports = set()
    symbols = body(".symtab")
    if symbols:
        header = sections[".symtab"]
        if (
            header[9] != 24
            or len(symbols) % 24
            or len(symbols) // 24 > 262144
            or not 0 < header[6] < count
        ):
            raise ValueError("invalid symbol table")
        strings_header = headers[header[6]]
        if strings_header[1] != 3:
            raise ValueError("invalid symbol strings")
        strings = region(data, strings_header[4], strings_header[5])
        for at in range(0, len(symbols), 24):
            name, info, _, section, _, _ = struct.unpack_from("<IBBHQQ", symbols, at)
            if section == 0 and name and info >> 4 in (1, 2):
                imports.add(string(strings, name))
    versions = {}
    raw_versions = body("__versions")
    if len(raw_versions) % 64:
        raise ValueError("unsupported modversion record size")
    for at in range(0, len(raw_versions), 64):
        crc = struct.unpack_from("<Q", raw_versions, at)[0]
        name = string(raw_versions[at + 8 : at + 64], 0)
        if not name or name in versions or crc > 0xFFFFFFFF:
            raise ValueError("invalid or duplicate modversion")
        versions[name] = f"{crc:08x}"
    return {
        "sha256": hashlib.sha256(data).hexdigest(),
        "bytes": len(data),
        "preflight_protocol": capability,
        "constructors": constructors,
        "imports": sorted(imports),
        "versions": dict(sorted(versions.items())),
        "extended_versions": bool(body("__version_ext_names") or body("__version_ext_crcs")),
    }


def compare(left: dict, right: dict) -> dict:
    common = sorted(set(left["imports"]) & set(right["imports"]))
    versioned = sorted(set(left["versions"]) & set(right["versions"]))
    return {
        "left_sha256": left["sha256"],
        "right_sha256": right["sha256"],
        "common_imports": common,
        "crc_evidence_available": bool(left["versions"] and right["versions"]),
        "common_version_records": len(versioned),
        "left_only_imports": sorted(set(left["imports"]) - set(right["imports"])),
        "right_only_imports": sorted(set(right["imports"]) - set(left["imports"])),
        "changed_crcs": {
            name: [left["versions"][name], right["versions"][name]]
            for name in versioned
            if left["versions"][name] != right["versions"][name]
        },
        "left_only_versions": sorted(set(left["versions"]) - set(right["versions"])),
        "right_only_versions": sorted(set(right["versions"]) - set(left["versions"])),
        "missing_import_crcs": {
            side: sorted(set(value["imports"]) - set(value["versions"]))
            for side, value in (("left", left), ("right", right))
        },
        "private_abi_proven": False,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path, nargs="?")
    args = parser.parse_args()
    left = inspect(args.left.read_bytes())
    report = left if args.right is None else compare(left, inspect(args.right.read_bytes()))
    print(json.dumps(report, indent=2, sort_keys=True))
