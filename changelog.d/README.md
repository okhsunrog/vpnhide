# Unreleased changelog fragments

Each `.md` file here is a single entry that will land in the next
release. `./scripts/release.py` rotates all fragments into `history[0]`
of `lsposed/app/src/main/assets/changelog.json` and deletes the files.

## Adding an entry

```sh
./scripts/changelog.py <type> "<EN text>" "<RU text>"
```

Types: `added`, `changed`, `fixed`, `removed`, `deprecated`, `security`.

This writes a single file `changelog.d/<type>-<slug>-<hex4>.md`. That's
the only file modified — `CHANGELOG.md` is regenerated only at release
time, so two concurrent PRs can't collide on it. The 4-char hex suffix
is random; two teammates picking the same slug still produce different
filenames.

Commit the new fragment alongside your code change. To preview the
pending (fragments-only) changelog, run `./scripts/preview-changelog.py`.

## Fragment file format

Filename: `<type>-<slug>-<hex4>.md` — the type is part of the name so
you can tell what kind of change a fragment is at a glance in the
directory listing.

Body: a date line on top, then two language sections. Renders directly
on GitHub — no TOML, no YAML frontmatter, no parser dependency.

```markdown
_2026-04-19_

## English

App no longer crashes when ...

## Русский

Приложение больше не падает когда ...
```

The date is in `YYYY-MM-DD` format, wrapped in `_…_` so GitHub renders
it as a subtle italic line. `changelog.py` writes today's date
automatically; when editing by hand, keep the format literal so the
parser recognises it.

Leading/trailing whitespace inside each language section is stripped
when loaded. Fragments are sorted chronologically by that date when
the Unreleased preview is rendered; ties fall back to filename order.

Prefer the `changelog.py` script for creating fragments; editing by
hand works but you need to match the filename pattern and section
headings exactly.
