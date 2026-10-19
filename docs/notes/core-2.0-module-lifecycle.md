> See also [core-2.0-activator-cutover.md](core-2.0-activator-cutover.md) — activators deleted.

# Core common module lifecycle (step 5)

Magisk-format module `id=vpnhide_core` under `core/module/`. Same zip format for
Magisk, KernelSU / KSU-Next, and APatch.

## Layout

```
core/module/
  module.prop
  META-INF/com/google/android/{update-binary,updater-script}
  customize.sh          # install binary, persist dir 0700
  post-fs-data.sh       # mkdir persist, claim config_owned early
  service.sh            # export production env, start vpnhide-core
  uninstall.sh          # restricted: stop socket/log only
  sepolicy.rule         # app→core UDS (manager-applied)
    vpnhide-core[.arm64|.armv7]  # shipped by release packaging
```

Persist dir: `/data/adb/vpnhide/core/` — journal, markers, `boot_status`,
`boot_phase`, socket, log. Mode `0700`. **APatch SuperKey is never touched.**

## Boot sequence

1. **post-fs-data** (all modules): core creates persist dir, writes `boot_phase`,
   claims `config_owned` so writers that honor the marker are fenced early.
   Legacy kmod `boot-load` scripts **exit 0** if core is present and not disabled.
2. **service** (late-start): core exports production writers
   (`VPNHIDE_KMOD_CTL`, `VPNHIDE_PORTS`, `VPNHIDE_BOOT_APPLY`, adopt/journal paths),
   starts `vpnhide-core $SOCKET` in the background.
3. Daemon **load_with_journal**: fence Running→Unknown, reconcile evidence, adopt.
4. **boot-apply** (`VPNHIDE_BOOT_APPLY=1`): `RepairActivation` — Native then Ports
   per env, no canonical rewrite. Result → `boot_status` (Applied / NoIntent /
   HeldUnknown / AppliedWithFailure / Refused).
5. Serve UDS + netlink observation. APK may be closed for the entire reboot.

Legacy backend `service.sh` scripts skip when `vpnhide_core` is installed and not
`disable`d (or when `config_owned` exists). Activator binaries also no-op on
resource markers.

## Disable

Manager creates `$MODDIR/disable`. Boot scripts are not run. **Already-applied**
kernel hooks and Ports rules remain (no unload). Diagnosable: no fresh
`boot_phase=service`, stale `boot_status`. Re-enable + reboot resumes apply.

## Uninstall

`uninstall.sh` removes socket/log/`boot_phase` only. **Retains** journal, markers,
`boot_status`, live iptables, kernel/KPM/Zygisk policy. Clearing
`/data/adb/vpnhide/core` is an explicit recovery/admin action — not part of
ordinary uninstall — so independent writers are not silently restored.

## Interrupted upgrades

`customize.sh` recreates persist dir without wiping journal/markers. service.sh
removes a stale socket before bind. If boot-apply Hits HeldUnknown, core serves
but refuses mutation until evidence resolve (explicit recovery).

## Missing / incompatible backends

Native delivery refuses foreign ctl owners; selection failures settle
FailedKnown and are written to `boot_status`. NoIntent (absent config) is
success-exit for `boot-apply` one-shot but logged — not silently “all good
protection”.

## Rollback

Flash previous core zip or disable module. Markers may still fence legacy until
manually removed — intentional dual-writer refuse. To roll back to activator-only:
disable/remove `vpnhide_core`, remove markers under persist (maintenance), reboot
so legacy `service.sh` runs again.

## Production write gating

Dev sandboxes leave `VPNHIDE_KMOD_CTL` / `VPNHIDE_PORTS` unset. **Installing this
module** is what enables those writers via `service.sh`. That is the packaging
gate; APK exclusive cutover (config_owned honor) remains a later step.
