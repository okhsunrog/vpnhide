# Diagnostic-thread RAW provider qualification — 2026-11-03

Historical cloud evidence, based on `2594bafa93db5d6fdf1941704332d8e04df56619`
plus the accompanying change. This qualifies the bounded `.ko` provider and
Linux Root/collector adapters, not the completed Android diagnostic flow.

## Implemented and exercised

- Separate private wire1, Rust-owned generation and independent C/Rust goldens;
  control2/telemetry1, RPC27 and journal13 stay unchanged.
- Root arm → actual thread self-registration → separate root activation. The
  retained PID object cannot be retargeted by numeric TID reuse. No UID table
  bypass or runtime RPC on hook calls.
- One fixed CLOCK_BOOTTIME scope; bounded nonspinning contention, nonrenewing
  replay, nonce-matching revoke, owning FD release and hot-path TTL expiry.
- Unprivileged registration uses an existing loopback socket ioctl; its ordinary
  name result is preserved. Private control remains root-only mode0600. No app
  proc SELinux permission expansion was added.
- Optional helper resolution uses real external kernel functions, not inline
  wrapper names, and introduces no mandatory KMI imports for those helpers.
  Unavailable capability does not change installed hooks/backend health.

The final protected QEMU session was
`9e9cb4404f8e1399b4772023db2a46acbea79e0e1e7f0e95aeb214f90a3bb46d`.
The real setuid Linux collector saw its owned TUN through RAW; another thread of
that same UID stayed protected. Revoke restored the original thread's filtering,
ordinary loopback survived, and a released nonce could not be resurrected.
The owner-close oracle bypassed Rust Drop's explicit revoke, testing actual final
FD release. The expiry observation happened before root readback/revoke/close,
testing deadline enforcement in the hot path itself.

The independent backend-free RAW boot was
`412a9030db754fba5d89321cba5e64c3ee2bc50f5000a6226dbdb88e7e927108`.
Existing interface/address/binding/proc fixtures and cleanup controls also ran.
Known proc-interface leaks remain retained failures, not concealment PASS.
The strengthened preflight oracle checked absence of the new private endpoint
alongside module/control/diagnostic nodes and unchanged probe registrations.

Checks: 447 selected Rust unit tests plus three netlink integration tests;
strict host and both Android ARM Clippy; C optimized and ASan/UBSan tests;
11 builtin integration safety tests; source export against actual 6.1 and
compilation of the current builtin control translation unit; Rust/C/Python
format/lint and workflow actionlint. No builtin boot or seven-KMI rebuild was
performed for this slice.

## Remaining code and acceptance

The production genuine Android worker still collects one protected native
sample with privileged root controls. It needs the admitted-thread RPC handshake,
PID/user/net namespace and birth revalidation, paired app RAW/protected samples
and assessment integration. KPM and Zygisk RAW adapters remain code work. This
provider evidence must not qualify those missing consumers or a full suite PASS.
Android bionic/SELinux/Zygote execution, actual builtin and mixed-ABI worker boots
remain acceptance gates after their respective code exists. No phone, provider,
reboot, production signing or public release operation was performed.

Machine-readable source/artifact identities and retained logs are in the cloud
workspace at `/workspace/vpnhide-build-evidence/rfc2/native-raw-provider/`.
Current work and remaining tasks belong to [ROADMAP](../../ROADMAP.md), not this
historical evidence note.
