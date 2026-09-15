#!/usr/bin/env python3
"""Generate docs/help/README.md — the GitHub-facing table of contents for the
offline user guide — from docs/help/manifest.json.

Single source: the manifest already drives the in-app guide, so this keeps the
repo index from drifting. Re-run after editing the manifest:

    python3 scripts/gen-help-index.py

Links point at the English article files (en/<id>.md); the ru/ and zh/ folders
hold the same ids. Stdlib-only, no dependencies.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
HELP = REPO / "docs" / "help"
MANIFEST = HELP / "manifest.json"
OUT = HELP / "README.md"

PRIMARY = "en"


def title(d: dict, key: str) -> str:
    label = d.get(key, {})
    return label.get(PRIMARY) or next(iter(label.values()), key)


def main() -> int:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    lines: list[str] = []
    lines.append("<!-- Generated from manifest.json by scripts/gen-help-index.py; do not edit. -->")
    lines.append("# VPN Hide — user guide")
    lines.append("")
    lines.append(
        "The offline help guide, shipped inside the app (**Settings → Help & guide**) "
        "and browsable here. Articles are localized: links below point at the English "
        "files; the same ids live under [`ru/`](ru/) and [`zh/`](zh/)."
    )
    lines.append("")

    for section in manifest["sections"]:
        lines.append(f"## {title(section, 'title')}")
        lines.append("")
        for article in section["articles"]:
            aid = article["id"]
            locales = article.get("locales", [PRIMARY, "ru", "zh"])
            note = ""
            if PRIMARY not in locales:
                # Regional article with no English file — link its first locale.
                first = locales[0]
                lines.append(f"- [{title(article, 'title')}]({first}/{aid}.md)")
                continue
            if "ru" not in locales:
                note = "  ·  _EN" + ("/ZH" if "zh" in locales else "") + " only_"
            lines.append(f"- [{title(article, 'title')}]({PRIMARY}/{aid}.md){note}")
        lines.append("")

    OUT.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
    count = sum(len(s["articles"]) for s in manifest["sections"])
    print(f"wrote {OUT.relative_to(REPO)} — {len(manifest['sections'])} sections, {count} articles")
    return 0


if __name__ == "__main__":
    sys.exit(main())
