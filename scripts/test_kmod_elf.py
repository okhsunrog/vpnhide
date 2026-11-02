"""Executable malformed-ELF and capability contracts for final ko inspection."""

import struct
import unittest

from kmod_elf import compare, inspect


def relocatable(modinfo=b"license=GPL\0", extra=(), attributes=None):
    contents = [(".modinfo", modinfo, 1), *extra]
    names = b"\0.shstrtab\0" + b"".join(name.encode() + b"\0" for name, _, _ in contents)
    sections = [(0, 0, 0, 0, 0, 0, 0, 0, 0, 0)]
    data = bytearray(64)
    data[:7] = b"\x7fELF\x02\x01\x01"
    struct.pack_into("<HHI", data, 16, 1, 183, 1)
    sections.append((1, 3, 0, 0, len(data), len(names), 0, 0, 1, 0))
    data.extend(names)
    for name, content, kind in contents:
        sections.append(
            (names.index(name.encode() + b"\0"), kind, 0, 0, len(data), len(content), 0, 0, 1, 0)
        )
        data.extend(content)
    struct.pack_into("<Q", data, 40, len(data))
    struct.pack_into("<HHH", data, 58, 64, len(sections), 1)
    for index, section in enumerate(sections):
        section = list(section)
        if index >= 2 and attributes:
            name = contents[index - 2][0]
            for at, value in attributes.get(name, {}).items():
                section[at] = value
        data.extend(struct.pack("<IIQQQQIIQQ", *section))
    return bytes(data)


def init_contract_payload(preflight=True):
    metadata = (
        b"vpnhide_init_contract=1\0parmtype=preflight_only:bool\0"
        b"parmtype=filesystem_hiding:bool\0parmtype=probe_mask:uint\0"
    )
    if preflight:
        metadata += b"vpnhide_preflight=1\0"
    strings = (
        b"\0param_ops_bool\0param_ops_uint\0__this_module\0"
        b"preflight_only\0filesystem_hiding\0probe_mask\0"
    )
    symbols = bytes(24)
    for name, binding, section in [
        ("param_ops_bool", 16, 0),
        ("param_ops_uint", 16, 0),
        ("__this_module", 17, 9),
        ("", 3, 7),
        ("", 3, 8),
    ]:
        symbols += struct.pack(
            "<IBBHQQ",
            strings.index(name.encode() + b"\0") if name else 0,
            binding,
            0,
            section,
            0,
            64 if name == "__this_module" else 0,
        )
    for name, value, size in [
        ("preflight_only", 0, 1),
        ("filesystem_hiding", 1, 1),
        ("probe_mask", 4, 4),
    ]:
        symbols += struct.pack(
            "<IBBHQQ", strings.index(name.encode() + b"\0"), 1, 0, 8, value, size
        )
    names = b"preflight_only\0filesystem_hiding\0probe_mask\0"
    relocations = b""
    for index, name in enumerate(["preflight_only", "filesystem_hiding", "probe_mask"]):
        for field, symbol, addend in [
            (0, 4, names.index(name.encode() + b"\0")),
            (8, 3, 0),
            (16, 1 if index < 2 else 2, 0),
            (32, 5, [0, 1, 4][index]),
        ]:
            relocations += struct.pack("<QQQ", index * 40 + field, (symbol << 32) | 257, addend)
    return relocatable(
        metadata,
        [
            ("__param", bytes(120), 1),
            (".symtab", symbols, 2),
            (".strtab", strings, 3),
            (".rela__param", relocations, 4),
            (".rodata", names, 1),
            (".data", bytes(8), 1),
            (".gnu.linkonce.this_module", bytes(64), 1),
        ],
        {
            ".symtab": {6: 5, 9: 24},
            ".rela__param": {6: 4, 7: 3, 9: 24},
            ".rodata": {2: 2},
            ".data": {2: 3},
            ".gnu.linkonce.this_module": {2: 3},
        },
    )


def malformed_init_contracts():
    valid = init_contract_payload()
    table = struct.unpack_from("<Q", valid, 40)[0]
    relocation = struct.unpack_from("<Q", valid, table + 6 * 64 + 24)[0]
    result = {}
    for name, mutations in {
        "module-alias": [(32, 3, 0)],
        "swapped-setters": [(56, 2, 0), (96, 1, 0)],
        "scalar-overrun": [(112, 5, 7)],
        "scalar-misaligned": [(112, 5, 3)],
        "other-scalar": [(32, 5, 1)],
    }.items():
        bad = bytearray(valid)
        for offset, symbol, addend in mutations:
            at = relocation + (offset // 40 * 4 + [0, 8, 16, 32].index(offset % 40)) * 24
            struct.pack_into("<QQQ", bad, at, offset, symbol << 32 | 257, addend)
        result[name] = bytes(bad)
    return result


class KmodElfTests(unittest.TestCase):
    def test_legacy_payload_and_random_substring_never_advertise_preflight(self):
        for payload in [relocatable(), relocatable(extra=[(".text", b"vpnhide_preflight=1\0", 1)])]:
            self.assertIsNone(inspect(payload)["preflight_protocol"])

    def test_exact_capability_requires_the_actual_parameter_and_no_constructors(self):
        marker = b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0"
        self.assertEqual(inspect(relocatable(marker))["preflight_protocol"], 1)
        for bad in [
            b"vpnhide_preflight=1\0",
            marker + b"vpnhide_preflight=1\0",
            marker.replace(b"preflight=1", b"preflight=2"),
            marker + b"parmtype=preflight_only:uint\0",
        ]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                inspect(relocatable(bad))
        with self.assertRaises(ValueError):
            inspect(relocatable(marker, [(".init_array", b"constructor", 1)]))

    def test_offsets_lengths_duplicate_sections_and_unterminated_metadata_are_rejected(self):
        valid = relocatable()
        for size in [0, 7, 63, len(valid) - 1]:
            with self.subTest(size=size), self.assertRaises(ValueError):
                inspect(valid[:size])
        bad_offset = bytearray(valid)
        struct.pack_into("<Q", bad_offset, 40, 2**64 - 1)
        wrong_kind = bytearray(relocatable(b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0"))
        table = struct.unpack_from("<Q", wrong_kind, 40)[0]
        struct.pack_into("<I", wrong_kind, table + 2 * 64 + 4, 3)
        for bad in [
            bytes(bad_offset),
            bytes(wrong_kind),
            relocatable(extra=[("x" * 257, b"", 1)]),
            relocatable(b"unterminated"),
            relocatable(extra=[(".modinfo", b"again\0", 1)]),
        ]:
            with self.assertRaises(ValueError):
                inspect(bad)

    def test_closed_init_contract_checks_actual_parameter_callbacks_independently(self):
        from pathlib import Path

        golden = init_contract_payload()
        self.assertEqual(
            Path(__file__)
            .resolve()
            .parents[1]
            .joinpath("fixtures/payloads/init-contract.elf")
            .read_bytes(),
            golden,
        )
        self.assertEqual(inspect(golden)["init_contract"], 1)
        self.assertEqual(inspect(init_contract_payload(False))["init_contract"], 1)
        self.assertIsNone(inspect(init_contract_payload(False))["preflight_protocol"])
        for bad in [
            relocatable(b"vpnhide_init_contract=1\0"),
            golden.replace(b"param_ops_bool", b"owned_ops_bool"),
            golden.replace(b"init_contract=1", b"init_contract=2"),
        ]:
            with self.subTest(bad=bad[:100]), self.assertRaises(ValueError):
                inspect(bad)
        table = struct.unpack_from("<Q", golden, 40)[0]
        for offset, value, fmt in [(table + 6 * 64 + 44, 2, "<I"), (table + 8 * 64 + 8, 7, "<Q")]:
            bad = bytearray(golden)
            struct.pack_into(fmt, bad, offset, value)
            with self.assertRaises(ValueError):
                inspect(bytes(bad))

    def test_exact_scalar_binding_rejects_shared_alias_setter_and_overrun_fixtures(self):
        from pathlib import Path

        root = Path(__file__).resolve().parents[1] / "fixtures/payloads"
        for name, bad in malformed_init_contracts().items():
            self.assertEqual((root / f"invalid-{name}.elf").read_bytes(), bad)
            with self.subTest(name=name), self.assertRaises(ValueError):
                inspect(bad)

    def test_crc_records_are_explicit_and_missing_evidence_is_preserved(self):
        def version(name, crc):
            return struct.pack("<Q", crc) + (name.encode() + b"\0").ljust(56, b"\0")

        left = inspect(relocatable(extra=[("__versions", version("symbol", 1), 1)]))
        right = inspect(relocatable(extra=[("__versions", version("symbol", 2), 1)]))
        self.assertEqual(compare(left, right)["changed_crcs"], {"symbol": ["00000001", "00000002"]})
        self.assertFalse(compare(left, right)["private_abi_proven"])
        absent = inspect(relocatable())
        self.assertEqual(absent["versions"], {})
        self.assertEqual(compare(absent, absent)["changed_crcs"], {})
        with self.assertRaises(ValueError):
            inspect(relocatable(extra=[("__versions", version("symbol", 1) * 2, 1)]))


if __name__ == "__main__":
    unittest.main()
