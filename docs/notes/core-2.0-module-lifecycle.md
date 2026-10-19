> See also [core-2.0-activator-cutover.md](core-2.0-activator-cutover.md) — activators deleted.

# Core common module lifecycle

Magisk-format module `id=vpnhide_core` under `core/module/`. Same zip format for
Magisk, KernelSU / KSU-Next, and APatch.

**No shell logic runs on the device.** Every path, flag, check and step is Rust in
`vpnhide-core`; the module scripts are the irreducible manager hooks, enforced
byte-for-byte by `core/tests/check_module_scaffold.sh` (CI "Module scaffold check").

## Layout

```
core/module/
  module.prop
  META-INF/com/google/android/{update-binary,updater-script}  # recovery-flash shim only
  customize.sh   # 3 unconditional lines: keep bin/$ARCH/vpnhide-core, chmod 0755
  service.sh     # exec "${0%/*}/vpnhide-core" service
  sepolicy.rule  # app→core UDS (manager-applied; a policy file, not a script)
  bin/{arm64,arm}/vpnhide-core   # shipped by core/build.py; customize.sh keeps one
```

There is no `post-fs-data.sh` and no `uninstall.sh`. Backend modules (`vpnhide_kmod`,
`vpnhide_kpm`, `vpnhide_zygisk`) ship **no script at all**: the managers' default
extraction installs the artifact (0644, which is all `finit_module` / KernelPatch /
the Zygisk loader need). Conflicts are not warned about at flash time — core reports
them at boot (typed `NativeBootStatus`) and the app shows them.

Persist dir: `/data/adb/vpnhide/core/` — journal, markers, `boot_status`,
`boot_phase`, `native_boot.json`, log. Mode `0700`. **APatch SuperKey is never touched.**

## Boot sequence (`vpnhide-core service`, `crates/core-daemon/src/service.rs`)

1. Create the persist dir (0700), write `boot_phase=service`, claim `config_owned`.
2. Export the production profile into its own process from compiled-in constants plus
   `MODDIR/..` (modules root): config/journal/owner paths, `/proc/vpnhide_ctl`, KPM lock,
   Zygisk module dir, Ports, app socket dir, root effects, boot apply.
3. Detach (fork + setsid; stdout/stderr appended to `core.log`) so the manager's
   late-start step returns at once.
4. **load_with_journal**: fence Running→Unknown, reconcile evidence, adopt.
5. **Native selection, once per boot** (`native_boot.rs`): scan kmod / KPM / built-in /
   Zygisk; load the single candidate (kmod via `finit_module(2)`, KPM via KernelPatch),
   or nothing on a conflict; retire Zygisk targets when Zygisk is not the selection.
6. **Boot passes** on their own thread: unbounded PackageManager wait, Native repair,
   netd `bw_OUTPUT` wait (≤60 s), Ports, Ports again at T+30 s → `boot_status`.
7. Serve UDS + netlink observation. The APK may be closed for the entire boot.

## Disable

Manager creates `$MODDIR/disable` and skips `service.sh`. **Already-applied** kernel
hooks and Ports rules remain (no unload). Diagnosable: no fresh `boot_phase=service`,
stale `boot_status`. Re-enable + reboot resumes.

## Uninstall

No hook. The manager removes the module dir; core simply never starts again. Journal,
markers, `boot_status`, `core.log`, live iptables and kernel/KPM/Zygisk policy are
retained (as before), so a reinstalled core recovers. Clearing `/data/adb/vpnhide/core`
is an explicit `FullReset` / admin action. A removed *backend* module's leftover state
is cleaned by core's next boot scan.

## Interrupted upgrades

A new zip lands in `modules_update/`; core reports it as `staged` (and a staged second
backend as "conflict after reboot") until the manager swaps it in at boot.
If boot-apply hits HeldUnknown, core serves but refuses mutation until evidence
resolves (explicit recovery).

## Production write gating

Host/dev runs (`vpnhide-core [socket]`) leave every writer off. Only the `service`
entry point — i.e. installing this module — enables them.
