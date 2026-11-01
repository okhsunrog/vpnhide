#!/usr/bin/env python3
"""Prepare CI signing from existing secrets only; never generate a replacement key."""

import base64
import binascii
import os
import subprocess
import sys
import tempfile
from pathlib import Path

SECRET_NAMES = {
    "KEYSTORE_BASE64": "ANDROID_KEYSTORE_BASE64",
    "KEYSTORE_PASSWORD": "ANDROID_KEYSTORE_PASSWORD",
    "KEY_ALIAS": "ANDROID_KEY_ALIAS",
}


def property_value(value: str) -> str:
    """ASCII Java Properties value, including whitespace, Unicode and backslashes."""
    encoded = value.encode("utf-16-be")
    return "".join(
        chr(n) if 33 <= n <= 126 and chr(n) not in "\\=:#!" else f"\\u{n:04x}"
        for n in (int.from_bytes(encoded[i : i + 2], "big") for i in range(0, len(encoded), 2))
    )


def main() -> int:
    missing = [name for key, name in SECRET_NAMES.items() if not os.environ.get(key)]
    if missing:
        print(
            f"::error::APK signing requires nonempty secrets: {', '.join(missing)}", file=sys.stderr
        )
        return 1
    temporary = None
    try:
        payload = base64.b64decode("".join(os.environ["KEYSTORE_BASE64"].split()), validate=True)
        if not payload:
            raise ValueError("empty decoded keystore")
        app = Path(os.environ["GITHUB_WORKSPACE"]) / "lsposed"
        with tempfile.NamedTemporaryFile(dir=app, delete=False) as file:
            temporary = Path(file.name)
            file.write(payload)
        result = subprocess.run(
            [
                "keytool",
                "-J-Duser.language=en",
                "-J-Duser.country=US",
                "-list",
                "-keystore",
                str(temporary),
                "-storepass:env",
                "KEYSTORE_PASSWORD",
                "-alias",
                os.environ["KEY_ALIAS"],
            ],
            capture_output=True,
            check=False,
        )
        if result.returncode or b"PrivateKeyEntry" not in result.stdout:
            raise ValueError("unusable signing entry")
        keystore = app / "release.jks"
        temporary.replace(keystore)
        temporary = None
        properties = app / "keystore.properties"
        values = {
            "password": os.environ["KEYSTORE_PASSWORD"],
            "keyAlias": os.environ["KEY_ALIAS"],
            "storeFile": str(keystore),
        }
        with os.fdopen(
            os.open(properties, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w"
        ) as file:
            os.fchmod(file.fileno(), 0o600)
            file.write("".join(f"{key}={property_value(value)}\n" for key, value in values.items()))
    except (ValueError, binascii.Error, OSError):
        # Neither keytool output nor secret values belong in CI logs.
        print(
            "::error::APK signing setup failed: invalid keystore, password, or private-key alias; "
            "no fallback key is generated.",
            file=sys.stderr,
        )
        return 1
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
