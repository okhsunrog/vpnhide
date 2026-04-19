# Releasing

## Model

- `VERSION` file = **the last released version** on `main`. It is only modified by `release.py`.
- `changelog.d/*.md` = work in progress. One Markdown file per unreleased entry, accumulated via `./scripts/changelog.py` during normal development. See [changelog.md](changelog.md).
- Intermediate builds (main, feature branches, local) get a version string derived from `git describe` — propagated into `module.prop` / APK `versionName` at build time. See [build versions](#build-versions) below.

## Cutting a release

1. Make sure everything you want in the release is merged to `main` and `changelog.d/` contains exactly the fragments that should appear in the release notes.
2. Run the release script with the new version:
   ```sh
   ./scripts/release.py 0.6.2
   ```
   Atomically:
   - rotates every fragment under `changelog.d/` into `history[0]` with `version: "0.6.2"` and deletes the fragment files,
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
4. Wait for CI to finish the build. CI creates a **draft** GitHub release with all artifacts attached and release notes extracted from `CHANGELOG.md` — review it on the Releases page and click **Publish release** when you're happy.
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

Update-json **must** be committed *after* the GitHub release is **published** (i.e. after you promote the draft to public). Magisk and KSU fetch these files to decide whether an update is available, then download the zip from the URL inside. Draft releases are private — their asset URLs require auth — so update-json must not point at them.

## Notes

- `versionCode` is derived automatically by `release.py` as `major*10000 + minor*100 + patch` (e.g. `0.6.2` → `602`).
- If `changelog.d/` is empty when you run `release.py`, it warns but proceeds — useful for version-only bumps.
- `release.py` refuses to release a version that already exists in `history[]`.

## Build versions

Every packaging step runs `./scripts/build-version.sh` to compute the version string stamped into the artifact:

- **On a release tag `vX.Y.Z`:** `X.Y.Z`
- **N commits after the nearest tag:** `X.Y.Z-N-gSHA` (the git describe format)
- **Working tree dirty:** additional `-dirty` suffix
- **No git / no tags:** falls back to the `VERSION` file

This string goes into:

- `module.prop` `version=...` (visible in the Magisk/KSU manager app)
- APK `versionName` (visible in Android Settings → Apps, diagnostic debug zip, `BuildConfig.VERSION_NAME`)
- Inside the zip filenames (only for release tags; dev artifacts in CI keep a stable name)

The committed `module.prop` files are **not** modified — `build-zip.sh` stages a copy, patches the version there, and zips. `lsposed/app/build.gradle.kts` evaluates `build-version.sh` at configure time and sets `versionName` dynamically.

`versionCode` stays at the value baked in by the last `release.py` run (monotonically increasing integer required by Android/Magisk).
