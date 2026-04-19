#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "rich",
# ]
# ///
"""Create a bilingual unreleased changelog fragment.

Usage:
  changelog.py <type> "<EN text>" "<RU text>" [--slug SLUG]

Types: added, changed, fixed, removed, deprecated, security

Writes a Markdown file to `changelog.d/<type>-<slug>-<hex4>.md`. Nothing
else. `CHANGELOG.md` is only regenerated at release time — that's what
keeps concurrent PRs from conflicting on it.

`release.py` rotates every fragment into `history[0]` and deletes them.

The slug defaults to the first few words of the English text; pass
`--slug` to override. A 4-char random hex suffix is appended so two
PRs that happen to pick the same slug still produce different
filenames and don't collide on merge.

To preview the pending (fragment-only) changelog locally without
writing anything, run `./scripts/preview-changelog.py`.
"""

from __future__ import annotations

import argparse
import re
import secrets
import sys
from datetime import date as _date
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from changelog_lib import (  # type: ignore[import-not-found]
    FRAGMENTS_DIR,
    VALID_TYPES,
)
from rich.console import Console

MAX_SLUG_WORDS = 6
MAX_SLUG_LEN = 50
_SLUG_SCRUB = re.compile(r"[^a-z0-9]+")


def auto_slug(text: str) -> str:
    """Lowercase, keep alphanumerics, collapse runs of anything else to a
    single dash, take the first few words. Non-Latin (Cyrillic) chars
    get stripped — we slugify the EN text where that's not an issue.
    """
    lower = text.lower()
    scrubbed = _SLUG_SCRUB.sub("-", lower).strip("-")
    words = scrubbed.split("-")[:MAX_SLUG_WORDS]
    slug = "-".join(w for w in words if w)[:MAX_SLUG_LEN]
    return slug or "entry"


def fragment_path(type_: str, slug: str) -> Path:
    """`<type>-<slug>-<hex4>.md`. The 4-char hex suffix is random so two
    teammates picking the same slug still produce different filenames —
    probability of collision is 1/65536 per pair of same-slug attempts,
    effectively zero for this project's volume.
    """
    FRAGMENTS_DIR.mkdir(parents=True, exist_ok=True)
    while True:
        suffix = secrets.token_hex(2)  # 4 hex chars
        candidate = FRAGMENTS_DIR / f"{type_}-{slug}-{suffix}.md"
        if not candidate.exists():
            return candidate


def write_fragment(path: Path, en: str, ru: str) -> None:
    today = _date.today().isoformat()
    body = f"_{today}_\n\n## English\n\n{en}\n\n## Русский\n\n{ru}\n"
    path.write_text(body, encoding="utf-8")


def main() -> int:
    console = Console()
    parser = argparse.ArgumentParser(description="Create an unreleased changelog fragment.")
    parser.add_argument("type", choices=VALID_TYPES)
    parser.add_argument("en", help="English text")
    parser.add_argument("ru", help="Russian text")
    parser.add_argument("--slug", help="custom slug (default: derived from EN text)")
    args = parser.parse_args()

    slug = args.slug or auto_slug(args.en)
    path = fragment_path(args.type, slug)
    write_fragment(path, args.en.strip(), args.ru.strip())

    console.print(f"[green]wrote[/green] {path.relative_to(Path.cwd())}")
    console.print(f"  [cyan]type:[/cyan] {args.type}")
    console.print(f"  [cyan]en:[/cyan]   {args.en}")
    console.print(f"  [cyan]ru:[/cyan]   {args.ru}")
    console.print(
        "\n[dim]commit just this fragment — CHANGELOG.md is regenerated on "
        "release only, so per-PR changes no longer conflict.[/dim]",
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
