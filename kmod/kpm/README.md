# vpnhide — KPM backend (KernelPatch Module)

A third native backend alongside the kretprobe `.ko`. Same job — hide VPN
interfaces from selected UIDs at the kernel level — for the kernels the
`.ko` **can't** serve.

## Why a KPM at all

The `.ko` needs per-GKI kernel headers + `Module.symvers` (the DDK build
matrix) and a kernel that allows loading unsigned modules. That leaves a
real gap:

- **Non-GKI / old kernels** — e.g. **4.14** ([#33](https://github.com/okhsunrog/vpnhide/issues/33)): no GKI KMI, no DDK build.
- **Proprietary kernels with no source or headers** — e.g. **HyperOS 5.4**
  (the crash report on [#35](https://github.com/okhsunrog/vpnhide/issues/35)): can't build a `.ko`, can't patch the source.
- **Locked-down module signing** — `insmod` rejected; a KPM is loaded via
  the KernelPatch supercall, not the module loader.

A KPM is compiled against KernelPatch's *own* headers (`-nostdinc`), needs
**no kernel headers and no `Module.symvers`**, resolves every kernel symbol
at load time via `kallsyms_lookup_name`, and installs **inline** hooks
(cheaper than kretprobe traps, with first-class arg/return rewriting).

This is **additive, not a replacement.** Mainstream GKI users stay on the
QEMU-tested `.ko` (no extra dependency). The KPM extends reach to the
segment above, at the cost of a KernelPatch runtime dependency.

## Architecture (and how it differs from the community prototypes)

```
kmod/
  ../data/interfaces.toml        # single source of truth (4 languages)
  generated/iface_lists.h        # shared VPN-name matcher  (incl. if<N>, #86)
  shared/vpnhide_logic.h         # shared filtering algorithms (freestanding)
  vpnhide_kmod.c                 # backend A: kretprobe .ko (unchanged)
  kpm/
    vpnhide_kpm.c                # backend C: KPM glue (hooks, proc, lifecycle)
    kver_offsets.h               # runtime per-version struct-offset table
    README.md                    # this file
```

| Layer | Shared? | Notes |
|---|---|---|
| VPN-name matcher | ✅ as-is | `generated/iface_lists.h`, from `data/interfaces.toml` |
| Filtering algorithms | ✅ `shared/vpnhide_logic.h` | seq-buffer compaction, UID parse — freestanding, included by both backends |
| Hook glue / struct access / proc | ❌ per-backend | incompatible include worlds (`<linux/*.h>` vs KernelPatch `-nostdinc`) |

Three deliberate improvements over [soranerai's prototype](https://github.com/soranerai/vpnhide/tree/kernelpatch-(outdated)) (which did the
legwork and tested on-device — credit due):

1. **One source + a runtime `kver` offset table** (`kver_offsets.h`), not
   three near-identical per-version files → one binary for 4.x/5.x/6.x.
2. **Per-call state via `fargs->local.dataN`**, not a per-CPU MPIDR stash
   (which races when a thread migrates between the before/after callback).
3. **Reuse the generated matcher** — gets the `if<N>` pattern (#86) the
   hardcoded community lists miss; and **don't hook `rt_fill_info`** (our
   QEMU harness proved its arg→register ABI is unstable).

## KernelPatch API used

- Module ABI: `KPM_NAME/VERSION/LICENSE/AUTHOR/DESCRIPTION`, `KPM_INIT`,
  `KPM_CTL0`, `KPM_EXIT` (`<kpmodule.h>`).
- Hooking: `hook_wrap(func, argno, before, after, udata)` /
  `hook_unwrap(...)`; callbacks get `hook_fargsN_t *` with `arg0..argN`,
  writable `ret`, `skip_origin`, and `local.data0..7` (`<hook.h>`).
- Symbols: runtime `kallsyms_lookup_name` (incl. static functions);
  `kfunc_match_cfi` for CFI jump tables (`<kallsyms.h>`, `<ksyms.h>`).
- Control: userspace talks to the module via the supercall (`syscall 45`)
  → `sc_kpm_control` → the module's `KPM_CTL0` handler.

## Build

A KPM is a relocatable object built with clang against the KernelPatch
header tree — **no kernel source needed**. The default build uses the pinned
`kmod/third_party/KernelPatch` submodule:

```sh
git submodule update --init kmod/third_party/KernelPatch
make -C kmod kpm
```

To test against a different KernelPatch checkout, override `KP_DIR`:

```sh
make -C kmod kpm KP_DIR=/path/to/KernelPatch
```

The flashable module zip is built by:

```sh
python3 kmod/kpm/build.py
```

That script builds the Android `kpm` activator, builds `vpnhide.kpm`, stages
`kmod/kpm/module/`, stamps `module.prop`, and writes `vpnhide-kpm.zip` at the
repo root. CI uses the same submodule-backed build.

## Deploy

A KPM runs on **inline hooks**, which physically require the KernelPatch
runtime (`kpimg`) to be embedded in the kernel — there is **no** "KPM on a
stock kernel". The `boot.img` always gets patched; the only question is which
root solution drives that patch. The `.kpm` itself is the same binary
everywhere (target the upstream `kpm.h` ABI).

Supported runtimes — pick whichever matches the device's root:

- **APatch / FolkPatch** — KernelPatch is built in; the flashable zip installs
  as an APatch/APM module, and the vpnhide activator loads/configures
  `vpnhide.kpm` through direct KernelPatch supercalls with the saved APatch
  SuperKey or the runtime's trusted `su` token when available.
- **KernelSU-Next** — flash **KPatch-Next** (one module, no switch to APatch).
- **Magisk or stock KernelSU** — flash the standalone
  [KPatch-Next-Module](https://github.com/KernelSU-Next/KPatch-Next-Module).
  It bundles `kpimg` + `kptools` + `magiskboot`, patches `boot.img` from its
  WebUI to embed KernelPatch, then (`service.sh`) auto-loads every
  `/data/adb/kp-next/kpm/*.kpm` on each boot via `kpatch kpm load`. So plain
  Magisk / KernelSU users get KPM support without changing root solution —
  the kernel is still patched, it's just automated. (Conflicts with APatch,
  which already ships KernelPatch.)

Persistence: a one-shot runtime `sc_kpm_load` is **lost on reboot**. The
vpnhide KPM module therefore ships `vpnhide.kpm` plus boot scripts: KPatch-Next
loads via its runtime `kpatch` CLI, while APatch/FolkPatch defers to the
activator and uses the saved SuperKey or trusted `su` token when present.

Targeting / control plane: our target-UID set is delivered via the module's
own `KPM_CTL0` supercall + load-args (the shape the QEMU harness exercises) —
this is independent of KPatch-Next's generic `package_config` →
`kpatch exclude_set <uid>` mechanism. The app stores package roles in
`/data/system/vpnhide_config.json`; the KPM activator resolves that canonical
config and pushes the same text wire through APatch/FolkPatch direct supercalls
or KPatch-Next `kpatch kpm ctl0`.

## Safety — read before testing on a device

Inline hooks have **no kprobe safety net**: a wrong offset in
`kver_offsets.h` or a mismatched kernel **corrupts kernel text → panic /
bootloop**, where the `.ko`'s kretprobe would just fail to register. So:

- **Every offset and every hook must pass the QEMU KPM harness first**
  (`../test/run-kpm.sh` — patches a KernelPatch kernel, boots it, runs the
  A/B vector assertions: target UID sees nothing, non-target sees `vpn0`, no
  panic). This harness is the whole reason the `.ko` is trusted; the KPM has
  the equivalent now, and a new offset table for a version is only as
  trustworthy as a green harness run on that version.
- Test with ephemeral **Load** (lost on reboot) before **Embed**.
- Patch the **inactive A/B slot** so a bad build falls back to the
  unpatched kernel.

## Status & backlog

- [x] Shared filtering logic extracted (`../shared/vpnhide_logic.h`)
- [x] KPM skeleton + 2 PoC hooks (`fib_route_seq_show`, `rtnl_fill_ifinfo`)
- [x] **Compiles against `bmax121/KernelPatch` → valid `.kpm` ELF** (self-contained matcher; `hook_fargs12_t` is the max bucket — rtnl reads arg0/arg1 only)
- [x] **Runtime `kver` detection** from KernelPatch's `kver` (common.h)
- [x] Target UIDs via load-args / `ctl0` supercall (reuses the shared parser)
- [x] **QEMU KPM harness** (`../test/run-kpm.sh`) — patches a GKI Image with
      KernelPatch, embeds the `.kpm`, boots under QEMU, two-boot A/B. **Both
      PoC hooks PASS on android12-5.10**: root sees `vpn0` when not targeted,
      not when targeted; no panic. Validates the inline hooks + the 5.x
      offsets (skb.len=104) + `fargs->local` state passing on a real kernel.
- [x] **All 10 hooks ported + QEMU-validated** on android12-5.10 (no panic),
      full native-vector parity with the `.ko`: `fib_route_seq_show`,
      `ipv6_route_seq_show`, `rtnl_fill_ifinfo`, `inet_fill_ifaddr`,
      `inet6_fill_ifaddr`, `dev_ioctl`, `sock_ioctl`, `fib_dump_info` (#86),
      `rt6_fill_node`, `fib_nl_fill_rule`. The deep-struct ones use 5.10
      offsets derived from source (`fib_info`, `fib6_info`, `inet6_ifaddr`,
      `fib_rule`); a static `getifaddrs()` probe (`gai-probe.c`) proves the
      address path is closed (target getifaddrs vpn0: 3 → 0).
- [x] Runtime kver offset table (`kver_offsets.h`) — **6.12 + 6.6 + 6.1 + 5.15 + 5.10 + 5.4 + 4.19 + 4.14, all full + QEMU-validated 9/9** (the complete Android kernel range, 8→16)
- [x] KernelPatch submodule under `kmod/third_party/KernelPatch`; `make kpm`
      works without an external checkout, and `KP_DIR=...` remains available
      for local experiments.
- [x] `kmod/kpm/build.py` packages the cross-version flashable KPM module zip.
- [x] CI KPM gates build the `.kpm` against the submodule and run the GKI +
      legacy QEMU KPM harnesses.
- [x] **6.12 (android16-6.12) — full parity, QEMU-validated 9/9** on a DDK GKI
      Image. Two version-specific changes vs 6.1: (a) `struct fib6_info` gained
      `gc_link` → `fib6_nh[]`@192; (b) the big **`struct net_device`
      cacheline-group reorg** moved `name` off offset 0 → `netdev_name`@296
      (derived by compiling `offsetof(struct net_device, name)` against the
      6.12 headers, then harness-confirmed). This is the first version where
      `name` isn't first, so the table's `netdev_name` field finally earns its
      keep — every name-based hook (link/addr/route dumps) depends on it.
- [x] **6.6 (android15-6.6) — full parity, QEMU-validated 9/9** on a DDK GKI
      Image. Every offset is byte-identical to 6.1 (unlike the 5.10→5.15 jump),
      so it shares the 6.1 table; the run just confirms it.
- [x] **5.15 (android13-5.15) — full parity, QEMU-validated 9/9** on a DDK GKI
      Image. Caught a latent bug: 5.15's `struct fib6_info` gained
      `offload`/`offload_failed` (like 6.1), so `nh`@160 / `fib6_nh`@176 — the
      old table silently routed 5.15 through 5.10's 152/168 and would misread
      the IPv6 route dev. Now its own entry; everything else matches 5.10.
- [x] **6.1 (android14-6.1) — full parity, QEMU-validated 9/9** on a DDK-built
      (clang) GKI Image. Derived from source: idev@168 (the `inet6_ifaddr`
      prefix is byte-identical to 5.10, so the on-device 216 first taken from
      soranerai was wrong — the `getifaddrs()` probe confirms 168), fib_info
      96/104/128 + fib_dump via `fib_rt_info*`@arg4 (= 5.10), fib6_info
      nh@160 / fib6_nh@176 (ANDROID_KABI_RESERVE before fib6_nh), skb.len@112.
- [x] **4.14 now full parity, QEMU-validated 9/9** (was 7). The oldest/most
      divergent target: IPv4 route dump via the legacy arg-9 fib_info (no
      nexthop objects); IPv6 route dump hooks `rt6_fill_node(struct rt6_info*)`
      — pre-`fib6_info`, so the dev is read straight from the embedded
      `dst_entry` (`rt6_via_dst`, dev@0); policy rules resolve through the
      `.isra` fuzzy fallback. `skb.len`@104 (older sk_buff head).
- [x] **4.19 (vanilla 4.19.325) — full parity, QEMU-validated 9/9.** Pre-nexthop
      kernel: fib_info/fib6_info have no `struct nexthop` field (the nh guards
      skip), fib_dump_info uses the legacy `<5.6` prototype (fib_info* at arg 9,
      shared with 5.4). Already has the `struct fib6_info` IPv6 model (4.14 does
      not). One config-sensitive offset: `fib6_nh.nh_dev` sits at +16 in
      `fib6_nh` (pre-`fib_nh_common`), and `CONFIG_IPV6_ROUTER_PREF` (Android-
      common, validated =y) shifts `fib6_nh` to @160 → dev@176.
- [x] **5.4 (android11-5.4) — full parity, QEMU-validated 9/9** on a from-source
      `5.4.302` Image. All vectors pass incl. both route dumps and policy rules.
      Notable per-version work that landed here:
      - `fib_dump_info` is the legacy `<5.6` prototype on 5.4 (fib_info passed
        *directly* at arg 9, not via `fib_rt_info`) — the hook is now
        table-driven (`fib_dump_fi_arg` / `fib_dump_fi_via_fri`) and unified on
        a 12-arg frame, so one callback serves 5.4 and 5.10.
      - `struct fib6_info` has no `ANDROID_KABI_RESERVE` here → `fib6_nh[]`@160
        (5.10 is 168); `inet6_ifaddr.idev`@168; `skb.len`@112 (conntrack on).
      - **Fuzzy symbol resolution**: gcc renames static fns to `name.isra.N` /
        `name.constprop.N`, which `kallsyms_lookup_name` misses. Hook lookups
        now fall back to the `name.`-prefixed clone (`kallsyms_on_each_symbol`),
        so e.g. `fib_nl_fill_rule.isra.21` is hooked. Clang device kernels keep
        the plain name (exact match wins first); this just hardens gcc kernels.
- [ ] proc_ops vs file_operations mock per kver (the HyperOS-5.4 crash class) — A/B currently uses load-args, so proc isn't on the critical path
- [ ] Confirm offsets on the closed-kernel targets with soranerai & cyberc3dr (real devices)
- [ ] Wire the `.ko` to `../shared/vpnhide_logic.h` (mechanical; gate on a local `.ko` harness run)

## Credits

- [soranerai](https://github.com/soranerai/vpnhide) — first working KPM ports (4.14/5.4/6.1) + on-device testing.
- [cyberc3dr](https://github.com/cyberc3dr/vpnhide-driver) — in-tree kernel patches + a real proprietary-kernel (HyperOS) test device.
- [bmax121/KernelPatch](https://github.com/bmax121/KernelPatch) & [KernelSU-Next/KPatch-Next](https://github.com/KernelSU-Next/KPatch-Next) — the runtime.
