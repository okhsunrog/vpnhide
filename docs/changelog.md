# Changelog

## Storage

Two locations:

- **`changelog.d/*.md`** — one Markdown file per unreleased entry. Each PR with a user-visible change adds its own file, so concurrent PRs don't touch the same bytes and don't conflict on the changelog. `release.py` rotates these files into the JSON and deletes them.
- **`lsposed/app/src/main/assets/changelog.json`** — bilingual (en/ru), released history only:

  ```json
  { "history": [
      { "version": "0.6.1", "sections": [...] },
      { "version": "0.6.0", "sections": [...] }
  ] }
  ```

  `history[0]` is the most recent released version. No `unreleased` field — those live in `changelog.d/` until a release promotes them.

### Fragment file format

Filename: `<type>-<slug>-<hex4>.md` (e.g. `fixed-dev-version-mismatch-a1b2.md`). The type (`added` / `changed` / `fixed` / `removed` / `deprecated` / `security`) is part of the name, so you can tell what a fragment is at a glance. The 4-char random hex keeps filenames collision-proof across concurrent PRs.

Body is plain Markdown — renders straight on GitHub, no YAML/TOML parser dependency:

```markdown
_2026-04-19_

## English

App no longer crashes when …

## Русский

Приложение больше не падает когда …
```

Date in `YYYY-MM-DD` format wrapped in `_…_` for subtle italic rendering. `changelog.py` writes today's date automatically. Fragments are sorted chronologically by that date for the Unreleased preview (ties → filename order). Leading/trailing whitespace inside each language section is stripped when loaded.

## Generated files (do NOT edit by hand)

Both markdown files are **only regenerated at release time** by `release.py`. Between releases they contain released versions only — the unreleased entries live in `changelog.d/`. This split is what keeps PRs from conflicting: if every PR regenerated `CHANGELOG.md`, two concurrent PRs would each write a different Unreleased block and collide.

- `CHANGELOG.md` at repo root — released history, Keep a Changelog format. CI extracts a single `## vX.Y.Z` block for the **GitHub release body**. Contains no `## [Unreleased]` block between releases.
- `update-json/changelog.md` — last 5 released versions only. Shown by Magisk/KSU in the update popup.

To see what's pending (not yet in `CHANGELOG.md`) run:

```sh
./scripts/preview-changelog.py    # prints to stdout, writes nothing
```

…or just browse `changelog.d/*.md` directly — each fragment renders as readable Markdown on GitHub.

## Adding an entry

From a PR branch:

```sh
./scripts/changelog.py <type> "<EN text>" "<RU text>"
```

Writes a Markdown fragment to `changelog.d/<type>-<slug>-<hex4>.md`. **Nothing else is modified** — no `CHANGELOG.md` regeneration, no `changelog.json` update. Commit just the new fragment alongside your code change. Two PRs doing this simultaneously produce two separate files and never conflict on merge.

Pass `--slug <slug>` to override the auto-derived slug. Filenames already carry a random 4-char hex suffix, so filename collisions are effectively impossible even when two PRs pick the same slug.

## When to add an entry

Add one for user-visible changes:

- new features / behaviour changes
- bug fixes that affect released versions
- security fixes
- breaking changes (also bump major/minor as appropriate)

Skip for: internal refactors with no behaviour change, documentation-only changes, CI/build tweaks, test additions.

## Cutting a release

See [releasing.md](releasing.md). The release script rotates every fragment under `changelog.d/` into `history[0]` atomically with the version bump and deletes the fragment files — no separate "rotate" step.
