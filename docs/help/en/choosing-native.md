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
- **Built-in (advanced, self-built kernels)** — an integration path for people
  compiling their own kernel with the project's patches. Ready-made kernels are
  not currently a supported distribution option. This is not a ZIP that adds
  support to a stock kernel. See the [developer integration guide](https://github.com/okhsunrog/vpnhide/tree/main/builtin).
  It needs no module: VPN Hide Core adopts the built-in driver at boot and delivers
  the configuration to it. A detected driver alone does not prove configuration is
  applied — check the Dashboard.

## Keep one native backend

Install one native backend. VPN Hide Core decides once per boot: if it finds more
than one — installed kmod, KPM or Zygisk modules, a kernel with VPN Hide built in,
or a VPN Hide KPM it did not load — it loads **none** of them, clears Zygisk's
target list, and the Dashboard names every backend involved. Remove the redundant
ones and reboot; core never picks one for you.

Never force `.ko` and KPM to run together; they hook the same kernel functions and
can freeze the device. With a self-built Built-in kernel, do not add `.ko` or KPM
on top.

**Java** and **Native** cover different detection paths. Both are recommended for
broader coverage, but a single layer can be used with reduced coverage. The Apps
role needs Java/LSPosed. See [Detection vectors](detection-vectors.md).

## Let the Dashboard pick

You don't have to decide by hand. Open the **Dashboard**: it detects your kernel
and recommends kmod, KPM, or Zygisk, with the exact file name to download from
the release. If a recommended backend won't load, the Dashboard says why and
what to try instead.
