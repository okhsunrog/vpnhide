# ADB root debugging

How to make `adb shell su -c ...` useful on rooted test devices, and how to
debug the common "root but Permission denied" failures.

This is test-device guidance. Do not ship broad diagnostic SELinux policy or
all-capability shell profiles as part of production modules.

## What "root" must include

For diagnostics, UID 0 is not enough. A useful adb root shell needs all three:

- UID/GID/groups that can pass Unix DAC checks.
- Linux capabilities, especially DAC override/read-search.
- SELinux allow rules for the file labels being read.

Check all three before assuming a device path is inaccessible:

```sh
adb -s SERIAL shell 'su -c "
id
getenforce
echo root_shell_pid=\$\$
grep -E \"Cap(Inh|Prm|Eff|Bnd|Amb)\" /proc/\$\$/status
ls -ldZ /data /data/adb /data/adb/modules /data/system
head -c 120 /data/system/vpnhide_config.json 2>&1
cat /data/adb/vpnhide_kpm/load_status 2>&1 | head
"'
```

## Command chains and quoting

When running more than one command through `su -c`, quote the whole remote
command for `adb shell`, then quote the payload passed to `su -c`.

Do not write chained commands like this:

```sh
adb -s SERIAL shell su -c 'id; iptables -S OUTPUT'
```

The local shell strips those quotes before `adb` sends the command, so Android's
remote shell can parse it as `su -c id; iptables -S OUTPUT`. Only `id` runs under
`su`; the rest runs as the ordinary adb shell (`uid=2000`, `u:r:shell:s0`,
usually with no useful capabilities).

Use one of these forms instead:

```sh
adb -s SERIAL shell "su -c 'id; iptables -S OUTPUT'"
adb -s SERIAL shell 'su -c "id; iptables -S OUTPUT"'
```

When checking the root shell itself, prefer `/proc/\$\$/status` inside the
`su -c` payload. `/proc/self/status` describes the process that opens the file
(`cat`, `grep`, etc.), which can hide quoting mistakes while debugging command
chains.

Healthy examples:

- KernelSU Next shell: `uid=0`, groups include `root`, `system`, `shell`, and
  `CapEff`/`CapPrm` are non-zero, for example `000001ffffffffff`.
- APatch shell: `uid=0`, SELinux context commonly `u:r:magisk:s0`, and
  `CapEff` is non-zero.

Broken examples:

- `uid=0(root)` but `CapEff: 0000000000000000`: KernelSU granted UID 0 but no
  capabilities. Root will still fail ordinary Unix permission checks such as
  `/data/local/tmp` or `0600` files.
- `ls -lZ` works but `cat` fails with `Permission denied`: SELinux allows
  metadata/stat but denies `open/read`.
- `/data/adb/ksu/bin/ksud: inaccessible or not found`: often an SELinux
  `execute`/`search` denial or a PATH/mount namespace issue, not a missing file.

## vpnhide paths worth checking

```sh
adb -s SERIAL shell 'su -c "
cat /data/system/vpnhide_config.json
cat /data/system/vpnhide_lsposed_state 2>/dev/null
cat /data/adb/vpnhide_kmod/load_status 2>/dev/null
cat /data/adb/vpnhide_kpm/load_status 2>/dev/null
cat /data/adb/vpnhide_ports/load_status 2>/dev/null
cat /data/adb/vpnhide_ports/load_log 2>/dev/null
ls -lZ /data/adb/modules/vpnhide_kmod /data/adb/modules/vpnhide_kpm /data/adb/modules/vpnhide_ports 2>&1
"'
```

The canonical JSON is `/data/system/vpnhide_config.json`, not `/data/adb`.
`/data/adb` is still important for module files, KPM status, superkey storage,
KernelSU/APatch CLIs, and LSPosed module state.

## APK variants for LSPosed/Vector smoke tests

Use the default debug APK (`:app:assembleDebug`) or the release APK for
cold-start checks. The default debug APK remains debuggable, but it is
R8/resource-shrunk so LSPosed/Vector has less dex to prepare after the APK is
updated as an Xposed module.

The `rawDebug` variant (`:app:assembleRawDebug`) keeps the old unminified debug
behavior for Studio/debugger work. It can be much larger; after updating that
APK, LSPosed/Vector may spend long enough preparing module code before the app
attaches that Android logs `Process ... failed to attach` / `start timeout`.
That symptom does not mean VPN Hide is in its own LSPosed scope. The expected
module scope is still **System Framework** only.

## KernelSU Next Shell profile

If `com.android.shell` has UID 0 but zero capabilities, fix the KernelSU Next
app profile:

1. KernelSU Next app -> Superuser -> Shell (`com.android.shell`).
2. Enable Superuser.
3. Use Custom profile.
4. Keep mount namespace `Inherited` unless debugging module mounts.
5. Set `uid = 0`, `gid = 0`.
6. Set groups to at least `root`, `system`, and `shell`.
7. For a dedicated test device, enable all capabilities. Minimal diagnostic
   subsets are easy to miss; all caps makes failures clearly SELinux-related.
8. Keep SELinux context as `u:r:ksu:s0` unless you know the target domain exists.

After changing the profile, open a new `adb shell su -c ...` session and verify:

```sh
adb -s SERIAL shell 'su -c "
id
echo root_shell_pid=\$\$
grep -E \"Cap(Prm|Eff|Bnd)\" /proc/\$\$/status
echo ok >/data/local/tmp/vpnhide_diag_write_test
cat /data/local/tmp/vpnhide_diag_write_test
rm /data/local/tmp/vpnhide_diag_write_test
"'
```

For SELinux read access in the `u:r:ksu:s0` domain, apply a narrow Shell policy
through KernelSU Next:

```sh
adb -s SERIAL shell 'cat >/data/local/tmp/vpnhide-ksu-shell.te <<EOF
allow ksu adb_data_file dir { getattr search open read }
allow ksu adb_data_file file { getattr open read map execute execute_no_trans }
allow ksu adb_data_file lnk_file { getattr open read }
allow ksu system_file dir { getattr search open read }
allow ksu system_file file { getattr open read map execute execute_no_trans }
allow ksu system_file lnk_file { getattr open read }
allow ksu system_data_file dir { getattr search open read }
allow ksu system_data_file file { getattr open read map }
allow ksu packages_list_file file { getattr open read map }
allow ksu shell_data_file dir { getattr search open read write add_name remove_name }
allow ksu shell_data_file file { getattr open read write create append unlink setattr map }
EOF'

adb -s SERIAL shell 'su -c "
PATH=/data/adb/ksu/bin:\$PATH
ksud sepolicy apply /data/local/tmp/vpnhide-ksu-shell.te
POLICY=\$(tr \"\n\" \";\" </data/local/tmp/vpnhide-ksu-shell.te)
ksud profile set-sepolicy com.android.shell \"\$POLICY\"
"'
```

`ksud sepolicy apply` is live. `ksud profile set-sepolicy` makes the same Shell
policy persistent across reboot.

Do not manually create random diagnostic directories under
`/data/adb/modules/` on KernelSU Next. Use app profiles or a properly installed
module zip. Broken module metadata can make `ksud module list` fail.

## APatch and Magisk-style shells

APatch shells usually run in `u:r:magisk:s0` and have non-zero capabilities by
default. If SELinux blocks `/data/adb` or `/data/system`, use `magiskpolicy`:

```sh
adb -s SERIAL shell 'cat >/data/local/tmp/vpnhide-magisk-shell.te <<EOF
allow magisk adb_data_file dir { getattr search open read }
allow magisk adb_data_file file { getattr open read map execute execute_no_trans }
allow magisk adb_data_file lnk_file { getattr open read }
allow magisk system_file dir { getattr search open read }
allow magisk system_file file { getattr open read map execute execute_no_trans }
allow magisk system_file lnk_file { getattr open read }
allow magisk system_data_file dir { getattr search open read }
allow magisk system_data_file file { getattr open read map }
allow magisk packages_list_file file { getattr open read map }
EOF'

adb -s SERIAL shell 'su -c "
PATH=/data/adb/ap/bin:/data/adb/magisk:\$PATH
magiskpolicy --live --apply /data/local/tmp/vpnhide-magisk-shell.te
"'
```

For persistence on APatch/Magisk test devices, install a small test-only module
with `sepolicy.rule`. APatch loads active modules' `sepolicy.rule` at boot.
Keep such modules out of release zips.

## KPM and KernelPatch diagnostics

Useful KPatch-Next checks:

```sh
adb -s SERIAL shell 'su -c "
PATH=/data/adb/modules/KPatch-Next/bin:/data/adb/ksu/bin:\$PATH
kpatch hello
kpatch kpm list
cat /data/adb/vpnhide_kpm/load_status
dmesg | grep -iE \"vpnhide|kpatch|kpm|KernelPatch\" | tail -n 80
"'
```

`kpatch kpm ctl0` exits with the KPM handler return value. Current vpnhide KPM
builds return `0` after applying config. Older builds returned the configured
target count, so `exit status: 15` can mean "15 targets configured", not
failure. Actual parse failures commonly surface as `255` because the KPM
returned `-1`.

## Quick recovery checklist

1. New `su` session, then check `id` and `CapEff`.
2. Check labels with `ls -lZ`.
3. Check whether the failing operation is DAC (`CapEff=0`, groups missing) or
   SELinux (`CapEff` non-zero, labels visible, `open/read/execute` denied).
4. On KernelSU Next, fix Shell groups/capabilities in the app profile first.
5. Apply only the SELinux rules needed for diagnostics.
6. Re-test `ksud module list`, vpnhide status files, and a write to
   `/data/local/tmp`.
