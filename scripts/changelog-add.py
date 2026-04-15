#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "rich",
# ]
# ///
"""Append a bilingual entry to the upcoming version's changelog.

Usage:
  changelog-add.py <type> "<EN text>" "<RU text>"

Types: added, changed, fixed, removed, deprecated, security

Writes both the JSON source and regenerates the markdown.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _changelog import (  # type: ignore[import-not-found]
    VALID_TYPES,
    append_entry,
    load_json,
    save_json,
    write_md,
)
from rich.console import Console


def main() -> int:
    console = Console()
    if len(sys.argv) != 4:
        console.print(
            "[red]usage:[/red] changelog-add.py <type> '<EN text>' '<RU text>'",
        )
        console.print(f"  types: {', '.join(VALID_TYPES)}")
        return 2
    type_, en, ru = sys.argv[1], sys.argv[2], sys.argv[3]
    if type_ not in VALID_TYPES:
        console.print(f"[red]error:[/red] invalid type {type_!r}")
        console.print(f"  valid: {', '.join(VALID_TYPES)}")
        return 2

    data = load_json()
    append_entry(data, type_, en, ru)
    save_json(data)
    write_md(data)

    console.print(
        f"[green]added[/green] \\[{type_}] entry to v{data['version']}",
    )
    console.print(f"  [cyan]en:[/cyan] {en}")
    console.print(f"  [cyan]ru:[/cyan] {ru}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
