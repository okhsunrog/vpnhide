# Experimental protection (filesystem paths)

**Settings → Experimental protection → Hide VPN filesystem paths** turns on an
extra hook group that hides the VPN interface's filesystem entries — paths like
`/sys/class/net/<iface>` and `/proc/sys/net/{ipv4,ipv6}/{conf,neigh}/<iface>`.

## You almost certainly don't need it

It's **off by default**, and standard Native protection is normally enough. No
known real-world app currently relies on this detection vector. Turn it on only
if you're specifically testing filesystem-path detection.

There's a real cost: on kernel backends these are **global VFS hooks** that can
slow filesystem operations for everyone, which is why they're opt-in and
reboot-gated.

## What it requires and how it behaves

- Needs an active **kmod, KPM, or Zygisk** backend.
- On **kmod / KPM** it intercepts the paths globally in the kernel (a stronger
  contract) and takes effect **after a reboot**.
- On **Zygisk** it's best-effort libc filtering inside the selected apps — raw
  syscalls and path aliases can bypass it — and takes effect after you **restart
  the selected apps**.

The Dashboard and this setting show whether the change is applied, pending a
reboot, or pending an app restart.

## Detection surfaces it doesn't cover

If Diagnostics reports a surface as *"Not covered,"* filesystem paths are one of
the things this setting closes. Others may need a kernel-level backend where your
device supports one — see
[Detection vectors and coverage](detection-vectors.md).
