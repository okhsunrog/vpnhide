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


def standard_parameters(data, sections, headers):
    """Contract1: exactly three standard scalar setters, no own pre-init callbacks."""
    try:
        params = sections["__param"]
        symbols = sections[".symtab"]
        relocations = sections[".rela__param"]
        strings = headers[symbols[6]]
    except (KeyError, IndexError) as error:
        raise ValueError("missing scalar parameter evidence") from error
    params_index = headers.index(params)
    symbols_index = headers.index(symbols)
    if (
        params[1] != 1
        or params[5] != 120
        or symbols[1] != 2
        or symbols[9] != 24
        or symbols[5] % 24
        or symbols[5] // 24 > 262144
        or strings[1] != 3
        or strings[5] > 16 * 1024 * 1024
        or relocations[1] != 4
        or relocations[5] != 12 * 24
        or relocations[9] != 24
        or relocations[6] != symbols_index
        or relocations[7] != params_index
    ):
        raise ValueError("unsupported scalar parameter layout")
    for header in headers:
        if header[1] in (4, 9) and header != relocations and header[7] == params_index:
            raise ValueError("multiple parameter relocation sources")
    raw = region(data, params[4], params[5])
    symtab = region(data, symbols[4], symbols[5])
    strtab = region(data, strings[4], strings[5])
    offsets, names, ops = set(), set(), []
    rows = {}
    for at in range(relocations[4], relocations[4] + relocations[5], 24):
        offset, info, addend = struct.unpack_from("<QQQ", data, at)
        if (
            offset >= 120
            or offset % 40 not in (0, 8, 16, 32)
            or offset in offsets
            or info & 0xFFFFFFFF != 257
            or region(raw, offset, 8) != bytes(8)
        ):
            raise ValueError("invalid parameter relocation")
        offsets.add(offset)
        symbol = region(symtab, (info >> 32) * 24, 24)
        name, binding, _, section, value, length = struct.unpack("<IBBHQQ", symbol)
        name = string(strtab, name, 256)
        kind = offset % 40
        row = rows.setdefault(offset // 40, {})
        if kind == 16:
            if (
                section != 0
                or binding >> 4 != 1
                or addend
                or name not in ("param_ops_bool", "param_ops_uint")
            ):
                raise ValueError("own or unknown parameter callback")
            ops.append(name)
            row["ops"] = name
        elif kind == 8:
            if (
                name != "__this_module"
                or addend
                or binding != 17
                or value
                or not 0 < section < len(headers)
                or headers[section] != sections.get(".gnu.linkonce.this_module")
                or not 0 < length <= headers[section][5]
            ):
                raise ValueError("invalid parameter module owner")
        else:
            if not 0 < section < len(headers):
                raise ValueError("invalid parameter target")
            target = headers[section]
            start = value + addend
            if start >= target[5] or target[2] & 4:
                raise ValueError("parameter outside safe data")
            if kind == 0:
                if target[1] == 8:
                    raise ValueError("parameter name in BSS")
                name = string(region(data, target[4], target[5]), start, 256)
                if (
                    name not in ("preflight_only", "filesystem_hiding", "probe_mask")
                    or name in names
                ):
                    raise ValueError("unexpected parameter name")
                names.add(name)
                row["name"] = name
            else:
                if (
                    target[1] not in (1, 8)
                    or not target[2] & 1
                    or target not in (sections.get(".data"), sections.get(".bss"))
                ):
                    raise ValueError("parameter argument is not writable scalar storage")
                row["argument"] = (section, start)
    if sorted(ops) != ["param_ops_bool", "param_ops_bool", "param_ops_uint"] or len(names) != 3:
        raise ValueError("incomplete scalar parameters")
    scalars = []
    for row in rows.values():
        name = row["name"]
        size = 4 if name == "probe_mask" else 1
        if row["ops"] != ("param_ops_uint" if size == 4 else "param_ops_bool"):
            raise ValueError("mismatched scalar setter")
        section, start = row["argument"]
        if start % size or start + size > headers[section][5]:
            raise ValueError("scalar crosses storage boundary")
        matches = []
        for at in range(0, len(symtab), 24):
            sn, info, _, ss, value, length = struct.unpack_from("<IBBHQQ", symtab, at)
            if string(strtab, sn, 256) == name:
                matches.append((info, ss, value, length))
        if matches != [(1, section, start, size)]:
            raise ValueError("parameter argument differs from its exact local scalar")
        if any(
            ss == section and start < other + length and other < start + size
            for ss, other, length in scalars
        ):
            raise ValueError("overlapping scalar storage")
        scalars.append((section, start, size))
        for header in headers:
            if header[1] in (4, 9) and header[7] == section:
                stride = 24 if header[1] == 4 else 16
                if header[9] != stride or header[5] % stride:
                    raise ValueError("invalid scalar storage relocation")
                for at in range(header[4], header[4] + header[5], stride):
                    offset = struct.unpack_from("<Q", data, at)[0]
                    if offset < start + size and offset + 16 > start:
                        raise ValueError("relocation overlaps parameter scalar")


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
    if ".modinfo" in sections and (not modinfo or not modinfo.endswith(b"\0")):
        raise ValueError("unterminated module metadata")
    records = [record.decode("ascii") for record in modinfo.split(b"\0") if record]
    protocol = [
        record.split("=", 1)[1] for record in records if record.startswith("vpnhide_preflight=")
    ]
    parameter = [record for record in records if record.startswith("parmtype=preflight_only:")]
    constructors = any(
        header[5] and (name in (".init_array", ".ctors", ".preinit_array") or header[1] in (14, 16))
        for name, header in sections.items()
    )
    capability = None
    if protocol:
        if protocol != ["1"] or parameter != ["parmtype=preflight_only:bool"] or constructors:
            raise ValueError("unsupported or inconsistent preflight capability")
        capability = 1

    init = [
        record.split("=", 1)[1] for record in records if record.startswith("vpnhide_init_contract=")
    ]
    init_contract = None
    if init:
        parameters = sorted(record for record in records if record.startswith("parmtype="))
        if (
            init != ["1"]
            or constructors
            or parameters
            != [
                "parmtype=filesystem_hiding:bool",
                "parmtype=preflight_only:bool",
                "parmtype=probe_mask:uint",
            ]
        ):
            raise ValueError("unverified closed init contract")
        standard_parameters(data, sections, headers)
        init_contract = 1

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
        "init_contract": init_contract,
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
