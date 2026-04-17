"""Shared helpers for changelog manipulation.

Used by `changelog.py` (append entry to unreleased) and `release.py`
(rotate unreleased into history + bump version files).

Source of truth: `lsposed/app/src/main/assets/changelog.json` (bilingual,
full history).

JSON schema:

```
{
  "unreleased": {
    "sections": [
      { "type": "fixed", "items": [{"en": "...", "ru": "..."}] }
    ]
  },
  "history": [
    { "version": "0.6.1", "sections": [...] },
    { "version": "0.6.0", "sections": [...] }
  ]
}
```

`unreleased` holds entries accumulated during development — it is
always present (possibly empty). `history[0]` is the most recent
released version.

Two generated markdown artifacts (en only, overwritten on every script
run — never edited by hand):

* `CHANGELOG.md` at the repo root — full history, Keep a Changelog
  convention with an optional `## [Unreleased]` block on top. The
  canonical human-facing changelog; CI extracts a single tag's section
  from here for the GitHub release body.
* `update-json/changelog.md` — the last MD_RECENT_VERSIONS released
  versions only (no Unreleased block). Served at a stable URL
  referenced from module update-json files; Magisk/KSU fetches it and
  displays it in the update popup.
"""

from __future__ import annotations

import json
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
JSON_PATH = REPO_ROOT / "lsposed/app/src/main/assets/changelog.json"
FULL_MD_PATH = REPO_ROOT / "CHANGELOG.md"
SHORT_MD_PATH = REPO_ROOT / "update-json/changelog.md"

VALID_TYPES = ("added", "changed", "fixed", "removed", "deprecated", "security")
MD_RECENT_VERSIONS = 5

_KEEP_A_CHANGELOG_HEADER = """# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

"""


def load_json() -> dict:
    return json.loads(JSON_PATH.read_text(encoding="utf-8"))


def save_json(data: dict) -> None:
    JSON_PATH.write_text(
        json.dumps(data, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def _section_items(entry: dict) -> list[tuple[str, list[dict]]]:
    """Return [(type, items), ...] for an entry in canonical order,
    skipping types with no items.
    """
    sections_by_type = {s["type"]: s for s in entry.get("sections", [])}
    out: list[tuple[str, list[dict]]] = []
    for type_ in VALID_TYPES:
        section = sections_by_type.get(type_)
        if section and section.get("items"):
            out.append((type_, section["items"]))
    return out


def _render_entry(heading: str, entry: dict, out: list[str]) -> None:
    out.append(f"## {heading}")
    out.append("")
    for type_, items in _section_items(entry):
        out.append(f"### {type_.title()}")
        for item in items:
            out.append(f"- {item['en']}")
        out.append("")


def render_full_md(data: dict) -> str:
    """Full history (with optional Unreleased block on top), Keep a
    Changelog header.
    """
    out: list[str] = []
    unreleased = data.get("unreleased", {"sections": []})
    if _section_items(unreleased):
        _render_entry("[Unreleased]", unreleased, out)
    for entry in data.get("history", []):
        _render_entry(f"v{entry['version']}", entry, out)
    return _KEEP_A_CHANGELOG_HEADER + "\n".join(out).rstrip() + "\n"


def render_short_md(data: dict) -> str:
    """Last MD_RECENT_VERSIONS released versions only, no Unreleased,
    no preamble. For Magisk/KSU popup.
    """
    out: list[str] = []
    for entry in data.get("history", [])[:MD_RECENT_VERSIONS]:
        _render_entry(f"v{entry['version']}", entry, out)
    return "\n".join(out).rstrip() + "\n"


def write_md(data: dict) -> None:
    FULL_MD_PATH.write_text(render_full_md(data), encoding="utf-8")
    SHORT_MD_PATH.write_text(render_short_md(data), encoding="utf-8")


def append_unreleased(data: dict, type_: str, en: str, ru: str) -> None:
    """Add an entry to the unreleased section."""
    if type_ not in VALID_TYPES:
        raise ValueError(f"invalid type {type_!r}; valid: {', '.join(VALID_TYPES)}")
    unreleased = data.setdefault("unreleased", {"sections": []})
    sections = unreleased.setdefault("sections", [])
    section = next((s for s in sections if s["type"] == type_), None)
    if section is None:
        section = {"type": type_, "items": []}
        sections.append(section)
    section["items"].append({"en": en, "ru": ru})


def rotate_unreleased(data: dict, version: str) -> dict:
    """Promote `unreleased` into `history[0]` with the given version,
    then reset `unreleased` to empty. Returns the newly-released entry.
    """
    unreleased = data.get("unreleased", {"sections": []})
    released = {
        "version": version,
        "sections": unreleased.get("sections", []),
    }
    history = data.setdefault("history", [])
    history.insert(0, released)
    data["unreleased"] = {"sections": []}
    return released


def unreleased_has_entries(data: dict) -> bool:
    return bool(_section_items(data.get("unreleased", {"sections": []})))
