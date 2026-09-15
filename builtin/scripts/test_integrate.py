"""Safety and evidence tests; no kernel builds or device access."""

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import integrate as module


class IntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.kernel = self.root / "kernel"
        self.kernel.mkdir()
        self.rules = {"android14-6.1": {"net/test.c": [("return 0;", "hook(); return 0;")]}}
        for name, text in {
            "Makefile": "VERSION = 6\nPATCHLEVEL = 1\n",
            "net/test.c": "int example(void) { return 0; }\n",
            "security/Kconfig": 'menu "Security"\nendmenu\n',
            "security/Makefile": "# build\n",
        }.items():
            module.put(self.kernel, name, text.encode())
        self.mock_rules = patch.object(module, "EDITS", self.rules)
        self.mock_rules.start()
        self.addCleanup(self.mock_rules.stop)

    def snapshot(self):
        return {
            str(p.relative_to(self.kernel)): p.read_bytes()
            for p in self.kernel.rglob("*")
            if p.is_file()
        }

    def test_export_preserves_dirty_tree_and_includes_new_sources(self):
        before = self.snapshot()
        report = module.integrate(self.kernel, "android14-6.1", self.root / "report")
        self.assertEqual(before, self.snapshot())
        self.assertEqual(report["checks"]["patch_round_trip"], "passed")
        content = (self.root / "report/vpnhide.patch").read_text()
        self.assertIn("new file mode", content)
        self.assertIn("security/vpnhide/shared/vpnhide_logic.h", content)
        self.assertEqual(
            report["files"]["net/test.c"]["before_sha256"], module.digest(before["net/test.c"])
        )
        self.assertTrue((self.root / "report/after/security/vpnhide/core.c").is_file())

    def test_failed_anchor_writes_failure_report_without_changing_tree(self):
        module.put(self.kernel, "net/test.c", b"return 0; return 0;\n")
        before = self.snapshot()
        with self.assertRaisesRegex(module.IntegrationError, "matched 2 times"):
            module.integrate(self.kernel, "android14-6.1", self.root / "report", apply=True)
        self.assertEqual(before, self.snapshot())
        report = json.loads((self.root / "report/report.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["edits"][0]["anchor_matches"], 2)

    def test_apply_and_reapply_requires_clean_baseline(self):
        module.integrate(self.kernel, "android14-6.1", self.root / "first", apply=True)
        before = self.snapshot()
        with self.assertRaisesRegex(module.IntegrationError, "Existing vpnhide"):
            module.integrate(self.kernel, "android14-6.1", self.root / "second", apply=True)
        self.assertEqual(before, self.snapshot())

    def test_changed_input_rejected_before_any_write(self):
        before, after = module.build_plan(self.kernel, "android14-6.1", {"edits": []})
        module.put(self.kernel, "net/test.c", b"concurrent change\n")
        changed = self.snapshot()
        with self.assertRaisesRegex(module.IntegrationError, "input changed"):
            module.apply_plan(self.kernel, before, after, {})
        self.assertEqual(changed, self.snapshot())

    def test_write_failure_rolls_back_prior_files(self):
        before, after = module.build_plan(self.kernel, "android14-6.1", {"edits": []})
        original = self.snapshot()
        real_replace = os.replace
        count = 0

        def fail_once(src, dest):
            nonlocal count
            count += 1
            if count == 2:
                raise OSError("simulated write failure")
            real_replace(src, dest)

        with (
            patch.object(module.os, "replace", fail_once),
            self.assertRaisesRegex(OSError, "simulated"),
        ):
            module.apply_plan(self.kernel, before, after, {})
        self.assertEqual(original, self.snapshot())

    def test_symlink_input_is_rejected(self):
        target = self.kernel / "net/test.c"
        target.unlink()
        target.symlink_to(self.kernel / "Makefile")
        with self.assertRaisesRegex(module.IntegrationError, "Symlink"):
            module.integrate(self.kernel, "android14-6.1", self.root / "report")

    def test_git_environment_cannot_redirect_private_index(self):
        outside = self.root / "outside-index"
        with patch.dict(os.environ, {"GIT_INDEX_FILE": str(outside)}):
            module.integrate(self.kernel, "android14-6.1", self.root / "report")
        self.assertFalse(outside.exists())

    def test_output_inside_kernel_is_rejected(self):
        before = self.snapshot()
        with self.assertRaisesRegex(module.IntegrationError, "outside"):
            module.integrate(self.kernel, "android14-6.1", self.kernel / "report")
        self.assertEqual(before, self.snapshot())

    def test_only_include_rules_can_use_a_narrower_anchor(self):
        self.rules["android14-6.1"]["net/test.c"] = [
            (
                "#include <one.h>\n#include <two.h>\n",
                "#include <one.h>\n#include <linux/vpnhide.h>\n#include <two.h>\n",
            )
        ]
        module.put(
            self.kernel, "net/test.c", b"#include <one.h>\n#include <susfs.h>\n#include <two.h>\n"
        )
        report = module.integrate(self.kernel, "android14-6.1", self.root / "report")
        self.assertEqual(report["edits"][0]["strategy"], "unique_preceding_include")
        self.assertIn("#include <susfs.h>", (self.root / "report/after/net/test.c").read_text())

    def test_wrong_kernel_version_is_rejected(self):
        module.put(self.kernel, "Makefile", b"VERSION = 6\nPATCHLEVEL = 6\n")
        with self.assertRaisesRegex(module.IntegrationError, "version mismatch"):
            module.integrate(self.kernel, "android14-6.1", self.root / "report")

    def test_source_checkout_can_be_reached_through_a_symlink(self):
        alias = self.root / "source"
        alias.symlink_to(module.REPO, target_is_directory=True)
        with patch.object(module, "__file__", str(alias / "builtin/scripts/integrate.py")):
            report = module.integrate(self.kernel, "android14-6.1", self.root / "report")
        self.assertIn("builtin/scripts/integrate.py", report["source_files"])


if __name__ == "__main__":
    unittest.main()
