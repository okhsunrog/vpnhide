# Requirements and compatibility

VPN Hide is a set of root modules plus this app. You need root and at least the
Java layer; the native layer needs a compatible kernel or a KernelPatch runtime.

## What you need

| Piece | Requires |
|---|---|
| App (picker) | Android 9+, arm64 or armv7 |
| Java layer | LSPosed, LSPosed-Next or Vector |
| Native — kmod | GKI kernel with `CONFIG_KPROBES` (standard on Android 12+) |
| Native — KPM | APatch or KPatch-Next-Module (a KernelPatch runtime) |
| Native — Zygisk | Zygisk (Magisk / KernelSU) or ZygiskNext |
| Ports (optional) | any root manager |

You install **one** native backend, the Java layer, and optionally Ports. The
app itself runs on 32-bit (armv7) devices too, but the kernel backends are arm64
only.

## Choosing a native backend

Prefer a kernel backend — the `kmod` or the built-in kernel patch — where your
device supports one: they hook in kernel space, so banking and payment apps can't
see them. `Zygisk` works wherever Zygisk does, but some of those apps detect its
in-process hooks. The Dashboard recommends a backend for your device.
