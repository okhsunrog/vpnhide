# Which native backend to use

The **native** backend hides the VPN from code that goes below the Java
framework — direct syscalls, `getifaddrs()`, netlink, `/proc/net`. There are a
few implementations of that same job. You install **exactly one**; the Dashboard
recommends the right one for your device and gives you its download name.

## The choices

- **Kernel module (Kmod)** — the default and most-tested backend. Hooks in
  kernel space, so it's invisible to anti-tamper checks that read the app's own
  memory. Needs a GKI kernel, and you install the ZIP matching your kernel's GKI
  generation. See [Install the kernel module](kmod-install.md).
- **KPM** — the same kernel-level hiding for kernels the `.ko` **can't** serve:
  old or non-GKI kernels, vendor kernels with no source, or kernels that reject
  unsigned modules. One universal module, but it needs a KernelPatch runtime.
  See [Install the KPM backend](kpm-install.md).
- **Zygisk** — a userspace fallback that hooks libc inside each target process.
  Works without a compatible kernel, but it's weaker: raw `svc` syscalls bypass
  it, and banking/payment apps may detect its in-process hooks. Use it when no
  kernel backend fits your device. See [Install the Zygisk module](zygisk-install.md).
- **Built-in** — the same kernel-level hiding compiled directly into a custom
  kernel (no loadable module). Only relevant if you build your own kernel from
  the project's patches; the Dashboard shows it as **Built-in** when present.

## Only one at a time

If more than one native backend is installed, only one runs — priority is
**kernel module → KPM → Zygisk** — and the others sit idle. Keep just the one
you use. In particular, **never run the kernel module and KPM together**: they
hook the same kernel functions, and having both active can hard-freeze the
device. The app warns you on the Dashboard when it sees a conflict.

The native backend always works alongside the **Java** layer (LSPosed), which
covers the framework APIs. Native and Java are two halves of the coverage, not
alternatives — see [Detection vectors and coverage](detection-vectors.md).

## Let the Dashboard pick

You don't have to decide by hand. Open the **Dashboard**: it detects your kernel
and recommends kmod, KPM, or Zygisk, with the exact file name to download from
the release. If a recommended backend won't load, the Dashboard says why and
what to try instead.
