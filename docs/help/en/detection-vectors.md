# Detection vectors and coverage

Apps have several ways to notice a VPN. Each maps to a VPN Hide role or backend.
This is the user-facing summary; the full per-backend matrix lives in the
project's developer docs.

| What an app can check | Covered by |
|---|---|
| `NetworkCapabilities`, `NetworkInfo`, `LinkProperties` (Java APIs) | Java (LSPosed) |
| The network-interface list (`getifaddrs` / `NetworkInterface`) | Native (kernel / libc) |
| Routing rules and netlink dumps | Native |
| A socket bound to the tunnel interface | Native |
| Installed VPN apps (package scan) | Apps |
| Localhost VPN / proxy ports | Ports |

## Notes

- **Java vs Native.** Some signals reach an app both through Java APIs and through
  libc/kernel calls. Java covers the first; a native backend covers the second —
  enabling both is the reliable choice.
- **Direct raw syscalls.** An app that calls the kernel directly (`svc #0`),
  bypassing libc, is only covered by a **kernel backend** (kmod / built-in / KPM),
  not by Zygisk's libc hooks.
- **SELinux.** On some ROMs a vector is already blocked by the system's SELinux
  policy — hidden, but not by VPN Hide, so it varies by device.
