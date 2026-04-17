# Changelog

## Source of truth

`lsposed/app/src/main/assets/changelog.json` — bilingual (en/ru), full history.

Schema:

```json
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

`unreleased` collects entries during development. `history[0]` is the most recent released version. The top-level structure is flat — there is no aspirational "upcoming version" field; the name is fixed only when `release.py` is run.

## Generated files (do NOT edit by hand)

Two markdown files are regenerated from the JSON by `scripts/changelog_lib.py`:

- `CHANGELOG.md` at repo root — full history, Keep a Changelog format. Renders `## [Unreleased]` on top when that section has entries, then each history entry as `## vX.Y.Z`. CI extracts a single `## vX.Y.Z` block for the **GitHub release body**, so don't edit release notes by hand either.
- `update-json/changelog.md` — last 5 **released** versions only (no Unreleased block). Shown by Magisk/KSU in the update popup inside the manager app.

## Adding an entry

From a PR branch:

```sh
./scripts/changelog.py <type> "<EN text>" "<RU text>"
```

Types: `added`, `changed`, `fixed`, `removed`, `deprecated`, `security`.

The entry lands in `unreleased.sections`. Both markdown files are regenerated automatically. Commit `lsposed/app/src/main/assets/changelog.json`, `CHANGELOG.md`, and `update-json/changelog.md` alongside your code change.

## When to add an entry

Add one for user-visible changes:

- new features / behaviour changes
- bug fixes that affect released versions
- security fixes
- breaking changes (also bump major/minor as appropriate)

Skip for: internal refactors with no behaviour change, documentation-only changes, CI/build tweaks, test additions.

## Cutting a release

See [releasing.md](releasing.md). The release script promotes `unreleased` into `history[0]` atomically with the version bump — there is no separate "rotate" step.
