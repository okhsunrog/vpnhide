"""Run the update generator and real producer packaging with compilers stubbed."""

import contextlib
import importlib.util
import io
import os
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

REPO = Path(__file__).resolve().parent.parent


def load(relative: str):
    spec = importlib.util.spec_from_file_location(relative.replace("/", "_"), REPO / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class UpdateDistributionTests(unittest.TestCase):
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
