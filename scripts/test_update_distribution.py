"""Run the update generator and real producer packaging with compilers stubbed."""

import contextlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlsplit

from public_distribution import PUBLIC_ASSETS

REPO = Path(__file__).resolve().parent.parent


def load(relative: str):
    spec = importlib.util.spec_from_file_location(relative.replace("/", "_"), REPO / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class UpdateDistributionTests(unittest.TestCase):
    def test_post_release_generator_updates_only_public_core_and_freezes_legacy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            shutil.copyfile(REPO / "scripts/update-json.sh", root / "scripts/update-json.sh")
            (root / "VERSION").write_text("2.0.0\n")
            feeds = root / "update-json"
            feeds.mkdir()
            old = {
                name: f"frozen old {name}\n"
                for name in (
                    "update-kmod-android14-6.1.json",
                    "update-kpm.json",
                    "update-zygisk.json",
                )
            }
            for name, body in old.items():
                (feeds / name).write_text(body)
            result = subprocess.run(
                ["bash", str(root / "scripts/update-json.sh")], capture_output=True
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            for name, body in old.items():
                self.assertEqual((feeds / name).read_text(), body)
            self.assertEqual({p.name for p in feeds.iterdir()} - old.keys(), {"update-core.json"})
            core = json.loads((feeds / "update-core.json").read_text())
            self.assertEqual(core["version"], "v2.0.0")
            self.assertEqual(core["versionCode"], 20000)
            asset = Path(urlsplit(core["zipUrl"]).path).name
            self.assertEqual(asset, "vpnhide-core.zip")
            self.assertIn(asset, PUBLIC_ASSETS)

    def test_invalid_version_writes_no_feed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            shutil.copyfile(REPO / "scripts/update-json.sh", root / "scripts/update-json.sh")
            (root / "VERSION").write_text("not-a-version\n")
            result = subprocess.run(
                ["bash", str(root / "scripts/update-json.sh")], capture_output=True
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / "update-json").exists())

    def test_internal_backend_producers_remove_retired_update_feeds(self):
        for relative in ("kmod/build.py", "kmod/kpm/build.py", "zygisk/build.py"):
            with self.subTest(producer=relative), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                module = load(relative)
                script = root / relative
                source = script.parent
                (source / "module").mkdir(parents=True)
                (source / "module/module.prop").write_text(
                    "id=fixture\nversion=v1.0.0\nupdateJson=https://example.invalid/retired.json\n"
                )
                destination = root / "input.zip"
                cwd = Path.cwd()
                try:
                    with contextlib.ExitStack() as stack:
                        stack.enter_context(patch.object(module, "__file__", str(script)))
                        stack.enter_context(
                            patch.object(module, "get_build_version", return_value="2.0.0")
                        )
                        stack.enter_context(patch.object(module.subprocess, "run"))
                        stack.enter_context(patch.object(module, "stamp"))
                        stack.enter_context(
                            patch.dict(
                                os.environ,
                                {"UPDATE_JSON_URL": "https://example.invalid/retired.json"},
                            )
                        )
                        stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
                        if relative == "kmod/build.py":
                            (source / "vpnhide_kmod.ko").write_bytes(b"compiler fixture")
                            result = module.native_build_one(
                                source, "android14-6.1", "fixture-kdir", None, destination
                            )
                        elif relative == "kmod/kpm/build.py":
                            (source.parent / "vpnhide.kpm").write_bytes(b"compiler fixture")
                            stack.enter_context(
                                patch("sys.argv", [str(script), "--out", str(destination)])
                            )
                            result = module.main()
                        else:
                            stack.enter_context(
                                patch.object(module, "detect_android_ndk", return_value=str(root))
                            )
                            for abi in ("aarch64-linux-android", "armv7-linux-androideabi"):
                                native = source / "target" / abi / "release/libvpnhide_zygisk.so"
                                native.parent.mkdir(parents=True)
                                native.write_bytes(b"compiler fixture")
                            result = module.main()
                            destination = source / "target/vpnhide-zygisk.zip"
                        self.assertEqual(result, 0)
                        with zipfile.ZipFile(destination) as archive:
                            properties = archive.read("module.prop").decode().splitlines()
                        self.assertIn("version=v2.0.0", properties)
                        self.assertFalse(any(line.startswith("updateJson=") for line in properties))
                finally:
                    os.chdir(cwd)


if __name__ == "__main__":
    unittest.main()
