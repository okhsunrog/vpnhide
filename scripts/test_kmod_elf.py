"""Executable malformed-ELF and capability contracts for final ko inspection."""

import struct
import unittest

from kmod_elf import compare, inspect


def relocatable(modinfo=b"license=GPL\0", extra=()):
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
    for section in sections:
        data.extend(struct.pack("<IIQQQQIIQQ", *section))
    return bytes(data)


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
        for bad in [
            bytes(bad_offset),
            relocatable(b"unterminated"),
            relocatable(extra=[(".modinfo", b"again\0", 1)]),
        ]:
            with self.assertRaises(ValueError):
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
