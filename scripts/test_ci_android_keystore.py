"""Failure paths must stop signing without leaking values or creating a substitute."""

import base64
import contextlib
import io
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from ci_android_keystore import main, property_value

SCRIPT = Path(__file__).with_name("ci_android_keystore.py")


class SigningTests(unittest.TestCase):
    def invoke(self, values):
        with tempfile.TemporaryDirectory() as root:
            (Path(root) / "lsposed").mkdir()
            env = {"PATH": os.environ["PATH"], "GITHUB_WORKSPACE": root, **values}
            result = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(list((Path(root) / "lsposed").iterdir()), [])
            return result.stderr.decode()

    def test_each_missing_secret_fails_without_outputs(self):
        values = {
            "KEYSTORE_BASE64": "invalid",
            "KEYSTORE_PASSWORD": "secret-marker",
            "KEY_ALIAS": "alias-marker",
        }
        for key in values:
            for present in (False, True):
                with self.subTest(key=key, present=present):
                    env = {name: value for name, value in values.items() if name != key}
                    if present:
                        env[key] = ""
                    message = self.invoke(env)
                    self.assertIn("requires nonempty secrets", message)
                    self.assertNotIn("secret-marker", message)
                    self.assertNotIn("alias-marker", message)

    def test_invalid_base64_never_generates_a_key(self):
        message = self.invoke(
            {
                "KEYSTORE_BASE64": "not-base64!",
                "KEYSTORE_PASSWORD": "secret-marker",
                "KEY_ALIAS": "alias-marker",
            }
        )
        self.assertIn("no fallback key", message)
        self.assertNotIn("secret-marker", message)

    def test_non_keystore_payload_fails_without_keytool_output(self):
        message = self.invoke(
            {
                "KEYSTORE_BASE64": base64.b64encode(b"not a keystore").decode(),
                "KEYSTORE_PASSWORD": "secret-marker",
                "KEY_ALIAS": "alias-marker",
            }
        )
        self.assertIn("no fallback key", message)
        self.assertNotIn("secret-marker", message)
        self.assertNotIn("alias-marker", message)

    def test_keytool_failure_or_certificate_only_entry_is_rejected(self):
        for code, output in [(1, b"sensitive error"), (0, b"trustedCertEntry")]:
            with self.subTest(code=code), tempfile.TemporaryDirectory() as root:
                app = Path(root) / "lsposed"
                app.mkdir()
                env = {
                    "GITHUB_WORKSPACE": root,
                    "KEYSTORE_BASE64": base64.b64encode(b"fixture").decode(),
                    "KEYSTORE_PASSWORD": "secret-marker",
                    "KEY_ALIAS": "alias-marker",
                }
                error = io.StringIO()
                result = subprocess.CompletedProcess([], code, stdout=output)
                with (
                    patch.dict(os.environ, env, clear=True),
                    patch("ci_android_keystore.subprocess.run", return_value=result),
                    contextlib.redirect_stderr(error),
                ):
                    self.assertEqual(main(), 1)
                self.assertEqual(list(app.iterdir()), [])
                self.assertNotIn("sensitive", error.getvalue())
                self.assertNotIn("secret-marker", error.getvalue())
                self.assertNotIn("alias-marker", error.getvalue())

    def test_properties_escape_java_special_characters(self):
        self.assertEqual(
            property_value(" a\\b\n:=#!"), r"\u0020a\u005cb\u000a\u003a\u003d\u0023\u0021"
        )
        self.assertEqual(property_value("я"), r"\u044f")


if __name__ == "__main__":
    unittest.main()
