"""Publisher identity covers a complete same-commit bundle, not a daemon base."""

import json
import unittest
import zipfile

import test_payload_bundle as bundle
from core_descriptor import descriptor, integer_constant


class DescriptorTests(unittest.TestCase):
    def setUp(self):
        self.fixture = bundle.BundleTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.build()

    def test_exact_assembled_package_and_current_compatibility(self):
        item = descriptor(self.fixture.output, bundle.COMMIT, 1001)
        self.assertEqual(item["package_bytes"], self.fixture.output.stat().st_size)
        self.assertEqual(item["source_commit"], bundle.COMMIT)
        self.assertEqual(item["build_sequence"], 1001)
        self.assertEqual(item["kernel_control"], 2)
        self.assertGreaterEqual(item["journal_max"], item["journal_min"])

    def test_base_missing_payloads_and_mixed_commit_are_refused(self):
        base = self.fixture.root / "vpnhide-core-base.zip"
        with self.assertRaises(KeyError):
            descriptor(base, bundle.COMMIT, 1001)
        for sequence in (0, -1, 2**64):
            with self.assertRaises(ValueError):
                descriptor(self.fixture.output, bundle.COMMIT, sequence)
        with self.assertRaises(ValueError):
            descriptor(self.fixture.output, "b" * 40, 1001)

    def test_incomplete_manifest_is_not_signed(self):
        changed = self.fixture.root / "incomplete.zip"
        with (
            zipfile.ZipFile(self.fixture.output) as original,
            zipfile.ZipFile(changed, "w") as output,
        ):
            for name in original.namelist():
                data = original.read(name)
                if name == "payloads/manifest.json":
                    item = json.loads(data)
                    item["payloads"].pop()
                    data = json.dumps(item)
                output.writestr(name, data)
        with self.assertRaises(ValueError):
            descriptor(changed, bundle.COMMIT, 1001)

    def test_ambiguous_constant_cannot_silently_choose_a_contract(self):
        source = self.fixture.root / "constant.rs"
        source.write_text("pub const VALUE: u32 = 1;\npub const VALUE: u32 = 2;\n")
        with self.assertRaises(ValueError):
            integer_constant(source, "VALUE")

    def test_duplicate_payload_identity_and_changed_payload_bytes_are_refused(self):
        for mutation in ("identity", "bytes", "path"):
            with self.subTest(mutation=mutation):
                changed = self.fixture.root / f"{mutation}.zip"
                with (
                    zipfile.ZipFile(self.fixture.output) as original,
                    zipfile.ZipFile(changed, "w") as output,
                ):
                    for name in original.namelist():
                        data = original.read(name)
                        if mutation == "identity" and name == "payloads/manifest.json":
                            item = json.loads(data)
                            item["payloads"][1] = item["payloads"][0]
                            data = json.dumps(item)
                        if mutation == "bytes" and name.endswith("vpnhide.kpm"):
                            data = b"different payload"
                        output.writestr(name, data)
                    if mutation == "path":
                        output.writestr("../escape", b"unsafe")
                with self.assertRaises(ValueError):
                    descriptor(changed, bundle.COMMIT, 1001)


if __name__ == "__main__":
    unittest.main()
