# Contributing to vpnhide

Thanks for your interest. This file covers the contribution **process**; for build and setup instructions see [docs/development.md](docs/development.md).

## Before you start

- For non-trivial changes, open an issue first to discuss the approach.
- Make sure your change builds locally and passes the CI lints listed in [docs/development.md](docs/development.md#ci-lints-run-before-pushing).

## Changes to existing behavior

Use the latest published 1.x release as the reference for existing user-facing
behavior during 2.x development. Keep that tag in a separate worktree and record the
tag and the relevant source paths or reproduction when comparing behavior.
Preserve the released behavior by default. Discuss intentional differences and
their user impact with the maintainer, who decides which changes to adopt.

Record those decisions in the relevant [RFC](docs/rfcs/README.md) as the work
progresses. Preserve the original proposal and update the current docs when the
agreed behavior changes in code.

## Commits

Use conventional-style prefixes, matching the existing history:

- `fix:` — bug fix
- `feat:` — new feature
- `refactor:` — code change without behaviour change
- `docs:` — documentation only
- `chore:` — tooling, release, CI, build
- `ci:` — CI-only changes

Scope the prefix where useful: `fix(zygisk): …`, `feat(lsposed): …`, `fix(kmod): …`.

Keep messages focused on *why*, not *what* — the diff already shows what changed.

## Changelog entry (required for user-visible changes)

Before opening a PR, add a changelog entry:

```sh
./scripts/changelog.py <type> "<EN text>" "<RU text>"
# types: added | changed | fixed | removed | deprecated | security
```

This writes a new Markdown fragment to `changelog.d/`. That's the only file changed — `CHANGELOG.md` is regenerated only at release time, which is what keeps concurrent PRs from conflicting. Commit just the fragment alongside your code change. Run `./scripts/preview-changelog.py` to see the pending entries together.

**Skip the entry** for internal refactors with no behaviour change, docs-only, CI-only, and test-only changes.

See [docs/changelog.md](docs/changelog.md) for the full changelog workflow.

## Pull requests

- Target the `main` branch.
- Keep PRs focused — one logical change per PR.
- Include a "why" in the PR description, and a brief "Testing" section if you verified anything manually that CI doesn't cover (hardware testing, specific device models, edge cases).
- CI must pass before merge.

## Code style

All style is enforced by CI — run the checks locally before pushing:

- Rust: `cargo fmt` + `cargo clippy -- -D warnings`
- C: `scripts/clang-format-c.sh --check` (all tracked kmod and builtin C sources)
- Kotlin: `ktlint`
- Android: `./gradlew :app:lint`

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE) that covers the project.
