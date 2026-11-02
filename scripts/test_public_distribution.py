"""Execute the same distribution policy used by the release job."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from public_distribution import select


class PublicDistributionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ("vpnhide.apk", "vpnhide-core.zip", "vpnhide-core.descriptor.json"):
            (self.root / name).write_bytes(b"producer-verified fixture")

    def test_backend_and_base_inputs_and_private_apks_are_not_published(self):
        for name in (
            "vpnhide-core-base.zip",
            "vpnhide-kmod-android14-6.1.zip",
            "vpnhide-kpm.zip",
            "vpnhide-zygisk.zip",
            "private-fixture.apk",
        ):
            (self.root / name).write_bytes(b"internal producer input")
        self.assertEqual(
            [p.name for p in select(self.root)],
            ["vpnhide.apk", "vpnhide-core.zip", "vpnhide-core.descriptor.json"],
        )

    def test_missing_or_empty_member_refuses_partial_distribution(self):
        for name in ("vpnhide.apk", "vpnhide-core.zip", "vpnhide-core.descriptor.json"):
            with self.subTest(name=name):
                path = self.root / name
                path.unlink()
                with self.assertRaises(ValueError):
                    select(self.root)
                path.write_bytes(b"")
                with self.assertRaises(ValueError):
                    select(self.root)
                path.write_bytes(b"restored fixture")

    def test_symlink_is_not_a_public_asset(self):
        path = self.root / "vpnhide-core.zip"
        path.unlink()
        path.symlink_to(self.root / "vpnhide.apk")
        with self.assertRaises(ValueError):
            select(self.root)

    def test_cli_exports_only_the_public_allowlist_to_actions(self):
        output = self.root / "github-output"
        script = Path(__file__).with_name("public_distribution.py")
        subprocess.run(
            [sys.executable, str(script), "--root", str(self.root), "--github-output", str(output)],
            check=True,
        )
        self.assertEqual(
            output.read_text().splitlines(),
            [
                "files<<VPNHIDE_PUBLIC_ASSETS",
                str(self.root / "vpnhide.apk"),
                str(self.root / "vpnhide-core.zip"),
                str(self.root / "vpnhide-core.descriptor.json"),
                "VPNHIDE_PUBLIC_ASSETS",
            ],
        )


if __name__ == "__main__":
    unittest.main()
