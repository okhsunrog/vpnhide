# core-2.0 activator cutover (hard pivot)

## What was deleted

- Per-backend activator binaries (`crates/activator` `[[bin]]` / `src/bin/*`)
- Boot/uninstall scripts that only invoked those activators (`service.sh`,
  `post-fs-data.sh`, `uninstall.sh` on kmod / KPM / Zygisk artifact modules)
- Dual-writer fencing (`activation_fenced_by_core`, `legacy-prefer-core.sh`)
- Companion Magisk module `builtin/module/` and `builtin/build.py` (`vpnhide-builtin.zip`)
- Entire `portshide/` packaging (`vpnhide-ports.zip`) — Ports apply lives in core-daemon

## What remains

| Surface | Role |
|---------|------|
| `vpnhide_core` Magisk module | **Required** — sole writer of config, ctl, Zygisk targets, Ports |
| `vpnhide-kmod-*.zip` | `.ko` artifact only |
| `vpnhide-kpm.zip` | `.kpm` artifact only |
| `vpnhide-zygisk.zip` | Zygisk `.so` only |
| `crates/activator` **lib** | Projection/delivery linked by core-daemon (not a process) |
| `builtin/` C + `scripts/integrate.py` | In-tree kernel driver sources (no Magisk companion) |

## Upgrade

1. Uninstall old activator-based modules (`vpnhide_builtin`, `vpnhide_ports`, and
   any kmod/kpm/zygisk module that still contains an `activator` binary).
2. Install **`vpnhide_core`** first.
3. Reinstall artifact zips from this release (artifact-only packages).
4. Reboot.

Do **not** run parallel activator + core. Dual-writer is unsupported.

## APK follow-up

Done. The app talks to core only over the socket through the in-process core-ffi
client (see `core-2.0-ipc-auth.md`): typed `CoreCommand` requests, core observations,
diagnostic probes and recording/lease APIs. SuperKey, import plus legacy cleanup
and full reset have durable compound receipts. `CoreConfigIo`, `ConfigRootIo`,
`suExec` and the `vhhelper` helper are
gone; CI (`lsposed/tests/check_no_root_exec.sh`) fails on any root exec in the APK.

The app also flags a still-installed `vpnhide_ports` / `vpnhide_builtin` module
on the Dashboard (`obsolete_modules` root-snapshot section).
