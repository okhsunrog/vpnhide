"""Shared helpers for changelog manipulation.

Used by `changelog.py` (create unreleased fragment), `release.py`
(rotate fragments into history + bump version files) and the
markdown-generation pipeline.

Storage layout:

* `lsposed/app/src/main/assets/changelog.json` — bilingual release
  history. Source of truth for everything that's already shipped.

      { "history": [ { "version": "0.6.1", "sections": [...] }, ... ] }

* `changelog.d/*.md` — one Markdown file per pending entry. Filename
  encodes the type and carries a short hex suffix for collision-proof
  uniqueness: `<type>-<slug>-<hex4>.md` (e.g.
  `fixed-dev-version-mismatch-a1b2.md`).

  Body is plain Markdown with two language sections — renders nicely
  straight on GitHub, no frontmatter, no YAML dependency:

      ## English

      App no longer crashes when ...

      ## Русский

      Приложение больше не падает когда ...

  Fragments live on disk and are accumulated across PRs. Because each
  entry is its own file, two PRs concurrently adding entries don't
  touch the same bytes and don't conflict. `release.py` rotates all
  fragments into `history[0]` and deletes them.

Generated (overwritten by `release.py` only — never by `changelog.py`,
so PRs don't write to these files and don't conflict):

* `CHANGELOG.md` at the repo root — Keep a Changelog, *released*
  history only. No `[Unreleased]` block — to preview pending fragments,
  run `./scripts/preview-changelog.py` (prints to stdout, never writes).
* `update-json/changelog.md` — last MD_RECENT_VERSIONS released
  versions. Served to Magisk/KSU update popups.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
JSON_PATH = REPO_ROOT / "lsposed/app/src/main/assets/changelog.json"
FULL_MD_PATH = REPO_ROOT / "CHANGELOG.md"
SHORT_MD_PATH = REPO_ROOT / "update-json/changelog.md"
FRAGMENTS_DIR = REPO_ROOT / "changelog.d"

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


_FRAGMENT_FILENAME = re.compile(r"^(?P<type>[a-z]+)-(?P<slug>.+)-(?P<hex>[0-9a-f]{4,})\.md$")
_DATE_LINE = re.compile(r"^_(\d{4}-\d{2}-\d{2})_\s*$", re.M)
_EN_HEADING = re.compile(r"^##\s+English\s*$", re.M)
_RU_HEADING = re.compile(r"^##\s+Русский\s*$", re.M)


def parse_fragment(path: Path) -> dict:
    """Parse a single fragment MD file. Raises ValueError on malformed
    input so callers can surface a meaningful error.
    """
    m = _FRAGMENT_FILENAME.match(path.name)
    if not m:
        raise ValueError(
            f"{path.name}: filename must be `<type>-<slug>-<hex>.md` "
            f"where <type> is one of {VALID_TYPES}",
        )
    type_ = m.group("type")
    if type_ not in VALID_TYPES:
        raise ValueError(f"{path.name}: type {type_!r} not in {VALID_TYPES}")

    text = path.read_text(encoding="utf-8")
    date_match = _DATE_LINE.search(text)
    if date_match is None:
        raise ValueError(
            f"{path.name}: body must start with a date line like `_YYYY-MM-DD_`",
        )
    date = date_match.group(1)

    en_match = _EN_HEADING.search(text)
    ru_match = _RU_HEADING.search(text)
    if en_match is None or ru_match is None:
        raise ValueError(
            f"{path.name}: body must contain both `## English` and `## Русский` sections",
        )
    if en_match.start() > ru_match.start():
        raise ValueError(f"{path.name}: `## English` must appear before `## Русский`")
    if date_match.start() > en_match.start():
        raise ValueError(f"{path.name}: date line must appear before the language sections")

    en_body = text[en_match.end():ru_match.start()].strip()
    ru_body = text[ru_match.end():].strip()
    if not en_body:
        raise ValueError(f"{path.name}: empty English section")
    if not ru_body:
        raise ValueError(f"{path.name}: empty Русский section")

    return {"path": path, "type": type_, "date": date, "en": en_body, "ru": ru_body}


def load_fragments() -> list[dict]:
    """Read every `*.md` under `changelog.d/` and sort chronologically
    by the embedded date (oldest first). Ties (same date) fall back to
    filename order for determinism.
    """
    if not FRAGMENTS_DIR.is_dir():
        return []
    fragments = [parse_fragment(p) for p in FRAGMENTS_DIR.glob("*.md") if p.name != "README.md"]
    fragments.sort(key=lambda f: (f["date"], f["path"].name))
    return fragments


def fragments_as_sections(fragments: list[dict]) -> list[dict]:
    """Group flat fragment list into the same shape used by history
    entries: `[{"type": T, "items": [{"en", "ru"}, ...]}, ...]`.
    Empty types are skipped. Order follows VALID_TYPES.
    """
    by_type: dict[str, list[dict]] = {}
    for fragment in fragments:
        by_type.setdefault(fragment["type"], []).append(
            {"en": fragment["en"], "ru": fragment["ru"]},
        )
    sections: list[dict] = []
    for type_ in VALID_TYPES:
        items = by_type.get(type_)
        if items:
            sections.append({"type": type_, "items": items})
    return sections


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
    """Full *released* history, Keep a Changelog header. The Unreleased
    block is deliberately NOT included: it would be regenerated by every
    PR and re-introduce CHANGELOG.md as a merge-conflict hotspot. To see
    what's pending, read `changelog.d/*.toml` directly or run
    `./scripts/preview-changelog.py`.
    """
    out: list[str] = []
    for entry in data.get("history", []):
        _render_entry(f"v{entry['version']}", entry, out)
    return _KEEP_A_CHANGELOG_HEADER + "\n".join(out).rstrip() + "\n"


def render_unreleased_md(fragments: list[dict]) -> str:
    """Unreleased-only preview, rendered on demand from fragments.
    Not written to a checked-in file.
    """
    sections = fragments_as_sections(fragments)
    if not sections:
        return "(no unreleased fragments)\n"
    out: list[str] = []
    _render_entry("[Unreleased]", {"sections": sections}, out)
    return "\n".join(out).rstrip() + "\n"


def render_short_md(data: dict) -> str:
    """Last MD_RECENT_VERSIONS released versions only, no Unreleased,
    no preamble. For Magisk/KSU popup.
    """
    out: list[str] = []
    for entry in data.get("history", [])[:MD_RECENT_VERSIONS]:
        _render_entry(f"v{entry['version']}", entry, out)
    return "\n".join(out).rstrip() + "\n"


def write_md(data: dict) -> None:
    """Regenerate the two checked-in markdown artifacts. Release-only —
    `changelog.py` (per-PR) does NOT call this, otherwise every PR would
    touch CHANGELOG.md and reintroduce conflicts.
    """
    FULL_MD_PATH.write_text(render_full_md(data), encoding="utf-8")
    SHORT_MD_PATH.write_text(render_short_md(data), encoding="utf-8")


def rotate_fragments_into_history(
    data: dict,
    fragments: list[dict],
    version: str,
) -> dict:
    """Promote the current fragment set into `history[0]` with the given
    version, then delete the fragment files. Returns the newly-released
    entry.
    """
    released = {
        "version": version,
        "sections": fragments_as_sections(fragments),
    }
    history = data.setdefault("history", [])
    history.insert(0, released)
    for fragment in fragments:
        fragment["path"].unlink()
    return released
