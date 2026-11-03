# Native address fixture qualification — 2026-11-03

Engineering evidence for the RPC25 / diagnostic-contract4 address slice. This is
an isolated AArch64 Linux QEMU test, not Android bionic, SELinux, Java, Zygisk,
ARM32 runtime or whole-product acceptance. No phone operations were performed.

## Executed scope

The actual Core TUN owner and shared observation/evaluation code ran through
`vpnhide-fixture-oracle` (the explicitly enabled `fixture-oracle` feature). The
oracle is not included in the Core or APK. Its app actor uses setuid only inside
the isolated guest; production diagnostics uses the Zygote-created Android worker.

The guest executor first configured ordinary loopback reference addresses. Core
then created its own DOWN, nonpersistent TUN and assigned exclusive IPv4 /32 and
IPv6 /128 addresses, without raising the link or requesting a prefix route.

| Run | Address result | Controls |
| --- | --- | --- |
| RAW, no backend | All 18 required subchecks leaked; none proven | No control failures |
| Protected, matching `.ko` | All 18 subchecks proven; no leaks | No control failures |

The address checks cover four named IPv4 ioctls, SIOCGIFCONF data and NULL-buffer
size, IPv4/IPv6 netlink through independent libc/syscall actors, plus IPv4/IPv6
getifaddrs. There is no fictitious syscall getifaddrs actor. The four earlier
interface checks and two socket-binding actors were checked separately.

Both runs also passed last-duplicate-FD, process-exit, post-allocation setup-fault
and **post-IPv4/pre-IPv6 setup-fault** controls. A retained FD prevented cleanup
confirmation and proof, while retaining leaks. A partial family setup remained
incomplete. After the final FD closed, the interface disappeared and the original
`/proc/net/route`, `/proc/net/ipv6_route` and `/proc/net/if_inet6` bytes returned.

The protected run passed the exact preflight sentinel with unchanged module,
proc and kprobe postconditions before loading the matching module. An earlier
attempt with a different kernel's `.ko` stopped at module-layout CRC refusal;
the harness did not dispatch its real load. This refusal is not counted as a
protected diagnostic run.

## Input identities

These are engineering inputs, not a newly assembled same-commit release bundle.

| Input | Bytes | SHA-256 |
| --- | ---: | --- |
| QEMU kernel | 25754112 | `c0472bd87ccc82dfa5e7d0a18a0333258d39ca1d2b425b8536a754a232cdd3f0` |
| Matching QEMU `.ko` | 97809 | `9f53c368170c1ac2efbfe46b12ce171f35153d5f1520c3fd59d6903f8120141e` |
| AArch64 musl guest executor | 1551040 | `45ce9b78e69d6168c6354ee52a26a0184bc05be136915a44ae4330869897e965` |
| AArch64 musl Core oracle | 752384 | `e49f268a534acf5afe5eac466b2fe0c19bd5cac8e6c3829a63be3baa06e4f739` |

Run sessions:

- RAW: `067efe31ef4e06023da6c7bf15bab28adeb2bb19653fcd0300ce583ccf18a5e9`.
- Protected: `df02a86be7bcb487c38cb63429ab6af147579b88675222ddb97a5a239c92469a`.

Retained cloud evidence is under
`/workspace/vpnhide-build-evidence/rfc2/addresses`, including serial logs, typed
plans, input identities and build/test logs. Reproduce through the typed
`vpnhide-tooling qemu run` runner with `VPNHIDE_QEMU_NATIVE_FIXTURE_MODE=raw` or
`protected`, explicitly supplying matching kernel/module, current static guest
and oracle, rootfs and iproute binary. The oracle requires the runner's explicit
kernel boot marker before it will create a fixture.

## Limits and next work

The test does not qualify KPM, a Zygisk-only backend, Android worker ABI mixing,
ordinary user traffic, callbacks or routes/rules hiding. Unavailable positive
stimuli, permissions and unstable reference lists remain unverified in Core.
Source inspection also finds a remaining KPM implementation gap:
`kmod/kpm/vpnhide_kpm.c::filter_ifconf` returns without adjusting NULL-buffer
length queries. The new independent size check must expose that gap; this `.ko`
result does not qualify KPM or remove that code task.
The required plan remains nineteen product vectors; address subchecks do not
replace the remaining proc/sys, route/rule and framework controls. There is still
no full-success diagnostic assessment variant. See the current
[diagnostic contract](../diagnostics.md) and [roadmap](../../ROADMAP.md).
