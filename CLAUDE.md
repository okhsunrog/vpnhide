# vpnhide — repo conventions for AI agents and contributors

Quick orientation file for anyone (or anything) working on this repo.
Read in full before opening a PR; it's short on purpose.

This file is also exposed as `AGENTS.md` (symlink) so vendor-neutral
tooling that follows the AGENTS-convention picks it up automatically.

## Project layout

Monorepo for hiding VPN interfaces from selected Android apps. Runtime components plus tooling:

- `kmod/` — kernel-level native backends: `.ko` kretprobe module per GKI generation, plus KPM beta for KernelPatch
- `zygisk/` — Rust Zygisk module, inline `libc` hooks via shadowhook
- `lsposed/` — LSPosed module + Compose target-picker app
- `portshide/` — localhost port blocker (shell + iptables)

## Read before touching code

These short files cover everything specific to this repo. Skipping them leads to broken commits and unnecessary review iterations.

- [CONTRIBUTING.md](CONTRIBUTING.md) — PR process, commit conventions, changelog requirement
- [docs/development.md](docs/development.md) — prereqs, per-module build quickstart, keystore setup, device install, CI lints
- [docs/state.md](docs/state.md) — every persistent path / proc entry / iptables chain the project touches; who writes, who reads, lifetime
- [docs/storage.md](docs/storage.md) — current storage & activation design: the single JSON canonical, the activator (Rust workspace, native/ports bins) that derives runtime state, LSPosed self-read, the APatch superkey, SELinux layout. Read before touching how config is stored/loaded.
- [docs/protocol.md](docs/protocol.md) — the frozen v1 control/stats **wire** between the app and the native backends (config/stats/status); the format every backend parser must agree on
- [docs/detection-vectors.md](docs/detection-vectors.md) — what an app can probe to detect the VPN (or a hidden package), which component (kmod / KPM / zygisk / lsposed / SELinux) covers each vector, how it manifests. Read before adding or changing a hook.
- [docs/adb-root-debugging.md](docs/adb-root-debugging.md) — how to make `adb shell su` useful on KernelSU/APatch/Magisk test devices; covers UID 0 without capabilities, SELinux denials on `/data/adb` and `/data/system`, and KPM/KPatch diagnostics.
- [docs/changelog.md](docs/changelog.md) — changelog storage (`changelog.d/` fragments + history JSON), `./scripts/changelog.py` usage
- [docs/releasing.md](docs/releasing.md) — `./scripts/release.py` usage, version-bump flow
- [kmod/BUILDING.md](kmod/BUILDING.md) — kernel-module build (one-command DDK via `./kmod/build.py`, GKI matrix, troubleshooting)
- [lsposed/AGENTS.md](lsposed/AGENTS.md) — Kotlin module: architecture, the load-bearing abstractions to reuse (don't reinvent), and quality gates (ktlint + detekt). Read before adding Kotlin code.

## Workflow rules

- **Add a changelog entry BEFORE committing user-visible changes:**
  ```sh
  ./scripts/changelog.py <type> "<EN text>" "<RU text>"
  # types: added | changed | fixed | removed | deprecated | security
  ```
  This writes a Markdown fragment to `changelog.d/<type>-<slug>-<hex4>.md` — nothing else. `CHANGELOG.md` is regenerated only at release time (that's what keeps PRs from conflicting on it). Commit just the new fragment alongside the code change. To preview pending entries: `./scripts/preview-changelog.py`. Skip the entry for internal refactors / docs-only / CI-only / test-only changes.
- **Do not bump `VERSION` or run `./scripts/release.py` unless the maintainer explicitly asks for a release.** Fragments under `changelog.d/` don't need a version number. `release.py` rotates every fragment into `history[0]` of `changelog.json`, deletes the fragment files, and is maintainer-only.
- **Don't put `#NN` in commit messages or PR titles to refer to local review notes.** GitHub auto-links `#NN` to PR/issue numbers in this repo, and the cross-reference will almost certainly point at the wrong PR. Real GitHub references (`fixes #38` where #38 is an actual issue) are fine — verify the number first.
- **Open regular PRs by default.** Use draft PRs only when the maintainer explicitly asks for a draft.
- **Do not mention Codex in PR titles, PR descriptions, commit messages, or changelog entries.**

## Build entry points

Single-command builds for both CI and local — the same scripts run in both places.

- **kmod**: `./kmod/build.py --kmi <kmi>` (or `--all`). Auto-spawns the DDK Docker/podman container if you're not already inside it; CI passes `--inside-container` to skip the spawn. The DDK image tag is `DDK_IMAGE_TAG` in `kmod/build.py` — `.github/workflows/ci.yml` mirrors it, bump both together.
- **KPM**: `python3 kmod/kpm/build.py` — builds the KernelPatch module and flashable `vpnhide-kpm.zip`.
- **zygisk**: `cd zygisk && ./build.py` — host-side cargo-ndk build, same script in CI image.
- **lsposed APK**: `cd lsposed && ./gradlew :app:assembleRelease`.

Both Rust cdylibs (`zygisk/build.rs`, `lsposed/native/build.rs`) pass `-Wl,-z,max-page-size=16384` so the resulting `.so` files load cleanly on 16 KiB-page Android devices (Pixel 8 Pro on Android 16, future hardware). Don't strip that flag.

## Reading device diagnostics from logcat

The picker app runs its full check suite (native probes + the Java VPN-presence checks, each PASS/FAIL with detail) at every cold start and logs every result to logcat under tag `VPNHideTest` — you do **not** need to open the Diagnostics screen to read them. But the app logger (`VpnHideLog`) is gated by the **Debug logging** toggle, which is off by default (stealth-first) and stored in the app's SharedPreferences — it can't be flipped reliably from `adb`/`su`.

So when you need diagnostics in the logs and Debug logging is off: **stop and ask the user to turn it on** (Diagnostics → Debug logging). Do not drive the GUI to read check results, and do not try to enable the flag by editing prefs via su. Once it's on (it persists across launches), capture is: `adb logcat -c` → cold-start the app (`am force-stop` + relaunch — checks run once per process and need an active VPN) → read the `VPNHideTest` / `VpnHide-Startup` lines.

## Design notes

- **KPM backend:** `kmod/kpm/` ships a KernelPatch Module beta packaged as `vpnhide-kpm.zip`. It is a kernel-level Native backend like the `.ko`, but uses KernelPatch inline hooks and one cross-version artifact for supported 4.14–6.12 kernels. It requires APatch or KPatch-Next-Module, and must not be active together with the `.ko`; supported GKI devices should still prefer the QEMU-tested `.ko` unless there is a real load/signing problem.
