# Releasing

## Model

- `VERSION` file = **the last released version** on `main`. It is only modified by `release.py`.
- `changelog.json.unreleased` = work in progress. Entries accumulate here via `./scripts/changelog.py` during normal development. See [changelog.md](changelog.md).
- Intermediate builds (main, feature branches, local) get a version string like `0.6.1-5-gabc1234` derived from `git describe` — propagated into `module.prop` / APK `versionName` at build time. (This is Phase 2 — tracked separately.)

## Cutting a release

1. Make sure everything you want in the release is merged to `main` and the `unreleased` section of `changelog.json` lists exactly what should appear in the release notes.
2. Run the release script with the new version:
   ```sh
   ./scripts/release.py 0.6.2
   ```
   Atomically:
   - promotes `unreleased` → `history[0]` with `version: "0.6.2"`,
   - resets `unreleased` to empty,
   - writes `0.6.2` to `VERSION`,
   - patches `versionName`/`versionCode` in every module.prop, Cargo.toml, and `build.gradle.kts`,
   - regenerates `CHANGELOG.md` and `update-json/changelog.md`.
3. Commit, tag, push:
   ```sh
   git commit -am "chore: release v0.6.2"
   git tag v0.6.2
   git push
   git push origin v0.6.2
   ```
4. Wait for CI to finish the build and publish the GitHub release.
5. Generate update-json files pointing at the new release assets:
   ```sh
   ./scripts/update-json.sh
   ```
6. Commit and push:
   ```sh
   git commit -am "chore: update-json for v0.6.2"
   git push
   ```

## Why update-json is a separate commit

Update-json **must** be committed *after* the GitHub release is live. Magisk and KSU fetch these files to decide whether an update is available, then download the zip from the URL inside. If update-json lands before the release exists, users see the new version but the download URL 404s.

## Notes

- `versionCode` is derived automatically by `release.py` as `major*10000 + minor*100 + patch` (e.g. `0.6.2` → `602`).
- If `unreleased` has no entries when you run `release.py`, it warns but proceeds — useful for version-only bumps.
- `release.py` refuses to release a version that already exists in `history[]`.
