# Glossary

Short definitions of terms used throughout this guide.

- **Backend** — one component that does the hiding. VPN Hide has a **Java**
  backend (LSPosed) and a **native** backend (kmod, KPM, Zygisk, or built-in).
  See [Which native backend to use](choosing-native.md).
- **Native** — hiding below the Java framework: kernel or libc level, covering
  direct syscalls, the interface list, netlink and `/proc/net`.
- **Java layer / LSPosed** — the framework used to hook `system_server` so the
  VPN is stripped from Android's Java network APIs before an app sees them.
- **Hook** — a single interception point. A backend installs several hooks, each
  covering one detection path.
- **Target** — an app you've selected to hide the VPN *from* (a bank, a store),
  as opposed to your VPN client, which is the thing being hidden.
- **Role (J / N / A / P)** — Java / Native / Apps / Ports, the four things you can
  turn on per target. See [Set up hiding](configure-hiding.md).
- **UID** — the Linux user ID Android assigns each app. Hiding is applied
  per-UID, which is how one app is filtered while the system and your VPN client
  are not.
- **Package** — an app's identifier (e.g. `com.example.bank`). Roles are stored
  by package name.
- **Profile** — a separate user space on the device: main profile, **work
  profile**, **cloned app**, or Second Space. Install VPN Hide in the main
  profile only — see [Work profiles](work-profiles.md).
- **GKI** — Google's Generic Kernel Image. The kernel module is built per GKI
  generation.
- **KMI** — Kernel Module Interface, the tag (e.g. `android14-6.1`) identifying a
  kernel's binary interface to modules. You match the `.ko` to it — see
  [Install the kernel module](kmod-install.md).
- **KernelPatch / SuperKey** — the runtime KPM needs, and the key APatch/FolkPatch
  uses to authorize it — see [Install the KPM backend](kpm-install.md).
- **Localhost / loopback** — the device's own `127.0.0.1` / `::1` address, where a
  local proxy or VPN daemon may listen. The **Ports** role blocks it per-app.
- **Routing table / route** — the kernel's list of where traffic goes; a VPN adds
  a route through its tunnel, which can give it away.
- **Split tunneling** — choosing, in your VPN client, which apps go through the
  tunnel and which go direct — see [Direct access or through the tunnel](split-tunneling.md).
- **SELinux** — Android's mandatory access control. If it's set to Permissive,
  several kernel-blocked detection paths open back up — see
  [What each result means](check-result-meanings.md).
