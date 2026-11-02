"""Assembly fixtures prove provenance and isolation, not kernel ABI compatibility."""

import hashlib
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

from payload_bundle import KMIS, PROVENANCE, Artifact, assemble
from test_kmod_elf import relocatable

COMMIT = "a" * 40


def elf(machine):
    header = bytearray(32)
    header[:6] = b"\x7fELF" + bytes([2 if machine == 183 else 1, 1])
    header[18:20] = machine.to_bytes(2, "little")
    return bytes(header)


def artifact(path, component_name, contents, **overrides):
    meta = {
        "schema": 1,
        "component": component_name,
        "source_commit": COMMIT,
        "dirty": False,
        "version": "test",
        "files": {
            name: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
            for name, data in contents.items()
        },
    }
    meta.update(overrides)
    with zipfile.ZipFile(path, "w") as archive:
        for name, data in contents.items():
            archive.writestr(name, data)
        archive.writestr(PROVENANCE, json.dumps(meta))


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "unified.zip"
        artifact(
            self.root / "vpnhide-core-base.zip",
            "core",
            {
                "module.prop": b"id=vpnhide_core\n",
                "bin/arm64/vpnhide-core": elf(183),
                "bin/arm/vpnhide-core": elf(40),
                "service.sh": b"exec core service\n",
            },
        )
        for kmi in KMIS:
            artifact(
                self.root / f"vpnhide-kmod-{kmi}.zip",
                f"kmod:{kmi}",
                {
                    "vpnhide_kmod.ko": relocatable(
                        b"vpnhide_preflight=1\0parmtype=preflight_only:bool\0"
                    ),
                    "module.prop": b"id=vpnhide_kmod\n",
                },
            )
        artifact(self.root / "vpnhide-kpm.zip", "kpm", {"vpnhide.kpm": elf(183)})
        artifact(
            self.root / "vpnhide-zygisk.zip",
            "zygisk",
            {
                "zygisk/arm64-v8a.so": elf(183),
                "zygisk/armeabi-v7a.so": elf(40),
                "module.prop": b"id=vpnhide_zygisk\n",
            },
        )

    def build(self):
        assemble(self.root, self.output, COMMIT)

    def test_legacy_or_inconsistent_ko_capability_refuses_before_output_replacement(self):
        self.build()
        previous = self.output.read_bytes()
        for metadata in [b"license=GPL\0", b"vpnhide_preflight=1\0"]:
            artifact(
                self.root / f"vpnhide-kmod-{KMIS[0]}.zip",
                f"kmod:{KMIS[0]}",
                {"vpnhide_kmod.ko": relocatable(metadata)},
            )
            with self.assertRaises(ValueError):
                self.build()
            self.assertEqual(self.output.read_bytes(), previous)

    def test_complete_passive_bundle_has_exact_hashes_and_no_backend_installers(self):
        self.build()
        with zipfile.ZipFile(self.output) as archive:
            names = archive.namelist()
            manifest = json.loads(archive.read("payloads/manifest.json"))
            self.assertEqual(len(manifest["payloads"]), 10)
            self.assertEqual(manifest["bundle_commit"], COMMIT)
            self.assertTrue(
                all(
                    p.get("preflight_protocol") == 1
                    for p in manifest["payloads"]
                    if p["implementation"]["kind"] == "kmod"
                )
            )
            self.assertFalse(any(name.startswith("zygisk/") for name in names))
            self.assertEqual([n for n in names if n.endswith("module.prop")], ["module.prop"])
            self.assertEqual(archive.read("module.prop"), b"id=vpnhide_core\n")
            for payload in manifest["payloads"]:
                self.assertEqual(payload["source_commit"], COMMIT)
                self.assertEqual(payload["control_version"], 2)
            self.assertEqual(
                archive.getinfo("bin/arm64/vpnhide-core").external_attr >> 16 & 0o777, 0o755
            )

    def test_every_missing_component_refuses_without_replacing_previous_output(self):
        self.output.write_bytes(b"previous good output")
        for source in list(self.root.glob("vpnhide-*.zip")):
            renamed = source.with_suffix(".saved")
            source.rename(renamed)
            try:
                with self.assertRaises(FileNotFoundError):
                    self.build()
                self.assertEqual(self.output.read_bytes(), b"previous good output")
            finally:
                renamed.rename(source)

    def test_wrong_source_dirty_and_component_are_refused(self):
        for overrides in [{"source_commit": "b" * 40}, {"dirty": True}, {"component": "other"}]:
            artifact(self.root / "vpnhide-kpm.zip", "kpm", {"vpnhide.kpm": elf(183)}, **overrides)
            with self.assertRaises(ValueError):
                self.build()
            self.assertFalse(self.output.exists())

    def test_wrong_abi_and_missing_32bit_library_are_refused(self):
        for files in [
            {"zygisk/arm64-v8a.so": elf(183)},
            {
                "zygisk/arm64-v8a.so": elf(183),
                "zygisk/armeabi-v7a.so": elf(183),
            },
        ]:
            artifact(self.root / "vpnhide-zygisk.zip", "zygisk", files)
            with self.assertRaises((ValueError, KeyError)):
                self.build()

    def test_tampered_hash_is_refused(self):
        artifact(
            self.root / "vpnhide-kpm.zip",
            "kpm",
            {"vpnhide.kpm": elf(183)},
            files={"vpnhide.kpm": {"bytes": 32, "sha256": "0" * 64}},
        )
        with self.assertRaises(ValueError):
            self.build()

    def test_unsafe_paths_and_symlinks_are_refused(self):
        for name in ["../escape", "/absolute", "a//b", "a/./b", "a\\b"]:
            artifact(self.root / "vpnhide-kpm.zip", "kpm", {name: elf(183)})
            with self.assertRaises(ValueError):
                self.build()
        artifact(self.root / "vpnhide-kpm.zip", "kpm", {"vpnhide.kpm": elf(183)})
        with zipfile.ZipFile(self.root / "vpnhide-kpm.zip", "a") as archive:
            info = zipfile.ZipInfo("link")
            info.external_attr = 0o120777 << 16
            archive.writestr(info, "vpnhide.kpm")
        with self.assertRaises(ValueError):
            self.build()

    def test_unlisted_member_and_autoload_in_base_are_refused(self):
        with zipfile.ZipFile(self.root / "vpnhide-kpm.zip", "a") as archive:
            archive.writestr("extra", "bad")
        with self.assertRaises(ValueError):
            self.build()
        artifact(self.root / "vpnhide-kpm.zip", "kpm", {"vpnhide.kpm": elf(183)})
        artifact(self.root / "vpnhide-core-base.zip", "core", {"zygisk/arm64-v8a.so": elf(183)})
        with self.assertRaises((ValueError, KeyError)):
            self.build()

    def test_size_budget_enforced_before_reading(self):
        # Tiny compressed input with an advertised oversized member; no large fixture needed.
        source = self.root / "vpnhide-kpm.zip"
        data = bytearray(source.read_bytes())
        central = data.index(b"PK\x01\x02")
        data[central + 24 : central + 28] = (129 * 1024 * 1024).to_bytes(4, "little")
        source.write_bytes(data)
        with self.assertRaises(ValueError):
            Artifact(source, "kpm", COMMIT)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--fixture":
        case = BundleTests()
        case.setUp()
        try:
            case.build()
            # Only the generated fixture above, never caller-provided ZIP contents.
            with zipfile.ZipFile(case.output) as archive:
                for name in archive.namelist():
                    destination = Path(sys.argv[2]) / name
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    destination.write_bytes(archive.read(name))
        finally:
            case.doCleanups()
    else:
        unittest.main()
