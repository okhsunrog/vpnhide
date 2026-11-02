#!/usr/bin/env python3
"""Execute the pinned fork's patch and offline package boundaries; no devices."""

import hashlib
import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("offline_package", BASE / "build_offline.py")
BUILD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD)
MODULE = Path(os.environ.get("VPNHIDE_KPATCH_MODULE_SRC", "/workspace/vpnhide-kpatch-upstream"))


class OfflinePackage(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vpnhide-fork-package-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        subprocess.run(
            ["git", "clone", "--quiet", "--shared", str(MODULE), str(self.source)], check=True
        )
        subprocess.run(
            ["git", "-C", str(self.source), "checkout", "--quiet", BUILD.MODULE_COMMIT], check=True
        )
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.manifest = {
            "module_commit": BUILD.MODULE_COMMIT,
            "engine_commit": BUILD.ENGINE_COMMIT,
            "tools": {},
        }
        elf = bytearray(64)
        elf[:6] = b"\x7fELF\x02\x01"
        elf[18:20] = (183).to_bytes(2, "little")
        # Non-executed ABI fixtures, never production/release tools.
        for name in BUILD.TOOLS:
            data = b"KP2026fixture" if name == "kpimg" else bytes(elf)
            (self.tools / name).write_bytes(data)
            self.manifest["tools"][name] = {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
                "source_commit": BUILD.ENGINE_COMMIT,
            }

    def patch(self):
        subprocess.run(
            [
                "git",
                "-C",
                str(self.source),
                "apply",
                str(BASE / "patches/0002-module-stage-tools-and-fence-shell-writes.patch"),
            ],
            check=True,
        )

    def test_requires_reviewed_patch_and_exact_hash_bound_tool_tuple(self):
        with self.assertRaises(ValueError):
            BUILD.prepare(self.source, self.tools, self.manifest)
        self.patch()
        files = BUILD.prepare(self.source, self.tools, self.manifest)
        self.assertEqual(
            set(files),
            set(BUILD.SAFETY_FILES) | {"provenance.json"} | {f"bin/{n}" for n in BUILD.TOOLS},
        )
        for name in BUILD.TOOLS:
            old = (self.tools / name).read_bytes()
            (self.tools / name).write_bytes(old + b"changed")
            with self.assertRaises(ValueError):
                BUILD.prepare(self.source, self.tools, self.manifest)
            (self.tools / name).write_bytes(old)
        (self.source / "module/post-mount.sh").write_text("dangerous additional entry point")
        with self.assertRaises(ValueError):
            BUILD.prepare(self.source, self.tools, self.manifest)

    def test_shell_entry_points_cannot_write_even_with_existing_stale_images(self):
        self.patch()
        target = self.root / "boot.img"
        target.write_bytes(b"original boot")
        (self.root / "kernel").write_bytes(b"stale kernel")
        (self.root / "new-boot.img").write_bytes(b"stale patched image")
        for script in ["boot_patch.sh", "boot_unpatch.sh", "boot_extract.sh"]:
            result = subprocess.run(
                ["sh", str(self.source / "module/patch" / script), str(target), "true"],
                cwd=self.root,
                capture_output=True,
                timeout=5,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(target.read_bytes(), b"original boot")
        result = subprocess.run(
            [
                "sh",
                "-c",
                '. "$1"; flash_image "$2" "$3"',
                "sh",
                str(self.source / "module/patch/util_functions.sh"),
                str(self.root / "new-boot.img"),
                str(target),
            ],
            cwd=self.root,
            capture_output=True,
            timeout=5,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(target.read_bytes(), b"original boot")

    def test_stage_install_never_hot_updates_active_module(self):
        self.patch()
        staged = self.source / "module"
        (staged / "provenance.json").write_text("{}")
        result = subprocess.run(
            [
                "sh",
                "-c",
                'abort() { exit 2; }; set_perm_recursive() { :; }; . "$1"',
                "sh",
                str(staged / "customize.sh"),
            ],
            env={**os.environ, "ARCH": "arm64", "MODPATH": str(staged), "APATCH": "true"},
            cwd=self.root,
            capture_output=True,
            timeout=5,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            (staged / "module.prop.bak").read_bytes(), (staged / "module.prop").read_bytes()
        )


if __name__ == "__main__":
    unittest.main()
