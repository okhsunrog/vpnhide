#!/usr/bin/env python3
"""Compile and execute the patched upstream config parser; no Android/device access."""

import gzip
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ENGINE_COMMIT = "456744b29efb9989445463ab29e368fa59a103c4"
ENGINE = Path(
    os.environ.get("VPNHIDE_KPATCH_ENGINE_SRC", "/workspace/vpnhide-kpatch-engine-upstream")
)


class ConfigDump(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if (
            subprocess.check_output(
                ["git", "-C", str(ENGINE), "rev-parse", "HEAD"], text=True
            ).strip()
            != ENGINE_COMMIT
        ):
            raise RuntimeError("engine source does not match the pinned commit")
        cls.temp = tempfile.TemporaryDirectory(prefix="vpnhide-kptools-test-")
        cls.work = Path(cls.temp.name)
        subprocess.run(
            ["git", "-C", str(ENGINE), "archive", "HEAD"],
            stdout=(archive := (cls.work / "source.tar").open("wb")),
            check=True,
        )
        archive.close()
        subprocess.run(["tar", "xf", str(cls.work / "source.tar"), "-C", str(cls.work)], check=True)
        subprocess.run(
            [
                "git",
                "apply",
                "--unsafe-paths",
                "--directory=" + str(cls.work),
                str(
                    ROOT
                    / "provisioning/kpatch-next/patches/0001-engine-validate-embedded-config.patch"
                ),
            ],
            cwd="/",
            check=True,
        )
        (cls.work / "test.c").write_text(r"""
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
extern int dump_all_ikconfig(char *, int32_t);
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    FILE *in = fopen(argv[1], "rb"); if (!in) return 2;
    if (fseek(in, 0, SEEK_END)) return 2;
    long len = ftell(in); if (len < 0 || len > 268435456) return 2;
    rewind(in); char *buf = malloc(len ? len : 1); if (!buf) return 2;
    if (fread(buf, 1, len, in) != (size_t)len) return 2;
    fclose(in); int rc = dump_all_ikconfig(buf, len); free(buf); return rc;
}
""")
        subprocess.run(
            [
                "cc",
                "-O1",
                "-ffunction-sections",
                "-fdata-sections",
                "-I" + str(cls.work / "tools"),
                str(cls.work / "tools/kallsym.c"),
                str(cls.work / "test.c"),
                "-Wl,--gc-sections",
                "-lz",
                "-o",
                str(cls.work / "dump"),
            ],
            check=True,
        )

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def dump(self, data):
        image = self.work / "Image"
        image.write_bytes(data)
        return subprocess.run(
            [str(self.work / "dump"), str(image)], cwd=self.work, capture_output=True, timeout=5
        )

    @staticmethod
    def member(config=b"CONFIG_KALLSYMS=y\n# CONFIG_KALLSYMS_ALL is not set\n"):
        return b"IKCFG_ST" + gzip.compress(config, mtime=0) + b"IKCFG_ED"

    def test_valid_config_is_exact_and_no_temp_file_is_followed(self):
        victim = self.work / "victim"
        victim.write_bytes(b"unchanged")
        (self.work / "temp.gz").symlink_to(victim)
        try:
            result = self.dump(b"prefix" + self.member() + b"suffix")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout, b"CONFIG_KALLSYMS=y\n# CONFIG_KALLSYMS_ALL is not set\n"
            )
            self.assertEqual(victim.read_bytes(), b"unchanged")
        finally:
            (self.work / "temp.gz").unlink()

    def test_invalid_streams_emit_no_config_prefix(self):
        valid = self.member()
        bad_crc = bytearray(valid)
        bad_crc[-16] ^= 1
        cases = [
            b"IKCFG_ED" + b"x" * 64 + b"IKCFG_ST",
            valid[:-1],
            valid + valid,
            bytes(bad_crc),
            self.member(b"CONFIG_KALLSYMS=y"),
            self.member(b"x" * (2 * 1024 * 1024 + 1)),
            b"IKCFG_STnotgzipIKCFG_ED",
            b"IKCFG_ST" + gzip.compress(b"CONFIG_KALLSYMS=y\n")[:-3] + b"IKCFG_ED",
        ]
        for data in cases:
            with self.subTest(size=len(data)):
                result = self.dump(data)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main()
