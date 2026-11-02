"""Select the public release files; producer artifacts remain internal CI inputs.

This allowlist does not publish anything or replace producer integrity/signature
checks. A release requires the complete app/Core pair and its signed descriptor.
"""

import argparse
from pathlib import Path

PUBLIC_ASSETS = ("vpnhide.apk", "vpnhide-core.zip", "vpnhide-core.descriptor.json")


def select(root: Path) -> list[Path]:
    result = []
    for name in PUBLIC_ASSETS:
        path = root / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"missing or invalid public asset: {name}")
        if any(c in str(path) for c in "\n\r\0"):
            raise ValueError("invalid public asset path")
        result.append(path)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--github-output", type=Path, required=True)
    args = parser.parse_args()
    try:
        files = select(args.root)
    except ValueError as error:
        parser.exit(1, f"{error}\n")
    with args.github_output.open("a", encoding="utf-8") as output:
        output.write("files<<VPNHIDE_PUBLIC_ASSETS\n")
        output.write("\n".join(str(path) for path in files))
        output.write("\nVPNHIDE_PUBLIC_ASSETS\n")


if __name__ == "__main__":
    main()
