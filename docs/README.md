# vpnhide documentation

Docs here are split by **audience**. Pick the row that matches what you're doing.

## For users — the offline guide

End-user documentation lives in **[`help/`](help/)** as localized Markdown
(EN/RU/ZH). This is the single source: the same articles are synced into the app
(**Settings → Help & guide**) and browsable here on GitHub. Start at the
**[guide index](help/README.md)**.

Covers install, choosing and installing a native backend, setting up hiding,
understanding the self-test results, statistics, updating/back-up/removal, limits,
and a glossary. The project **[README](../README.md)** is the short entry point
that links into it.

## For contributors — architecture & reference

Stable references for anyone changing the code. Read the relevant one before a PR.

| Doc | What it covers |
|---|---|
| [development.md](development.md) | Prereqs, per-module build quickstart, device install, CI lints |
| [protocol.md](protocol.md) | The frozen control/telemetry wire between activator, app and backends |
| [storage.md](storage.md) | Storage & activation design: the JSON canonical, the activator, SELinux |
| [state.md](state.md) | Every persistent path / proc entry / iptables chain the project touches |
| [detection-vectors.md](detection-vectors.md) | What an app can probe and which component covers each vector |
| [diagnostics.md](diagnostics.md) | How the app self-tests hiding and attributes the result |
| [debug-bundle.md](debug-bundle.md) | How to read a user's `vpnhide_debug_*.zip` bundle |
| [limits.md](limits.md) | Every target/config/stats capacity ceiling and what binds first |
| [lsposed-hook-debugging.md](lsposed-hook-debugging.md) | Diagnosing why a Java-layer hook didn't attach on a ROM |
| [adb-root-debugging.md](adb-root-debugging.md) | Making `adb shell su` useful on KernelSU/APatch/Magisk test devices |
| [avd-magisk-testing.md](avd-magisk-testing.md) | Testing on a rooted x86_64 AVD (Magisk + Zygisk + LSPosed) |
| [changelog.md](changelog.md) | Changelog storage (`changelog.d/` fragments) and `scripts/changelog.py` |
| [releasing.md](releasing.md) | `scripts/release.py` usage and the version-bump flow |

Module-specific developer docs live next to their code:
[kmod/BUILDING.md](../kmod/BUILDING.md), [kmod/kpm/README.md](../kmod/kpm/README.md),
[lsposed/AGENTS.md](../lsposed/AGENTS.md), and each module's `README.md`.
Repo conventions and the PR/changelog process are in
[CONTRIBUTING.md](../CONTRIBUTING.md) and [AGENTS.md](../AGENTS.md).

## Roadmap, plans & working notes

Forward-looking, non-reference material — directions, plans and TODOs that aren't
a release commitment — lives in **[`notes/`](notes/)**, starting with
[notes/ROADMAP.md](notes/ROADMAP.md).
