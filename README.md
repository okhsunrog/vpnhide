# VpnHide

An LSPosed / Vector (JingMatrix fork) module that hides an active VPN
connection from selected Android apps, without needing to actually turn the
VPN off.

Useful if you run a split-tunnel VPN (where only some traffic goes through
the tunnel) but certain apps still refuse to work when they see that *any*
VPN interface is up on the device.

> **Status:** experimental / personal-use. Tested baseline is crDroid 12.8
> (Android 16) on a Pixel 8 Pro with KernelSU-Next (LKM) + NeoZygisk +
> JingMatrix Vector, but the module itself is just an LSPosed/Vector
> hook plugin and runs on any LSPosed/Vector v93+ deployment regardless
> of the underlying root provider — Magisk, KernelSU, KernelSU-Next,
> APatch, with or without ZygiskNext / NeoZygisk / Magisk's built-in
> Zygisk.

> **Companion module:** this LSPosed module covers the Java / Android
> framework side. For the **native** detection path — apps that check
> for a VPN from C/C++/JNI/Flutter via `libc::ioctl`, `getifaddrs()`,
> `/proc/net/*` and never enter ART — use the matching Zygisk module
> [okhsunrog/vpnhide-zygisk](https://github.com/okhsunrog/vpnhide-zygisk).
> The two modules are independent and share no runtime state; you can
> install either one alone, but for full coverage of both the Java and
> native stacks you want both installed together.

### Verified against third-party detection apps

With **this module + [okhsunrog/vpnhide-zygisk](https://github.com/okhsunrog/vpnhide-zygisk)**
installed, and WireGuard running in **split-tunnel** mode (so the
detection apps' own HTTPS probes go out through the carrier, not the
tunnel), the following popular Russian "is there a VPN on this device?"
apps report **all clean**, with no direct or indirect signals
triggered:

- [xtclovver/RKNHardering](https://github.com/xtclovver/RKNHardering) —
  the Kotlin app implementing the Russian Ministry of Digital
  Development's VPN-detection methodology. All GeoIP, IP comparison,
  Direct signs (`TRANSPORT_VPN`, HTTP/SOCKS proxy), Indirect signs
  (`NET_CAPABILITY_NOT_VPN`, interface enumeration, MTU, default route,
  DNS servers, `dumpsys`), Location signals and Split-tunnel bypass
  cards come back Clean.
- [loop-uh/yourvpndead](https://github.com/loop-uh/yourvpndead) — the
  "no root, no permissions, standard Android API, under one second"
  detector. Reports `VPN: Не активен`; the only visible interfaces are
  `dummy0` / `lo` / `rmnet16`; no VPN signals in direct or indirect
  checks.

Neither module alone covers all of this:

- **This** module handles the Java / Android framework side:
  `NetworkCapabilities` (`hasTransport` / `hasCapability` /
  `getTransportInfo`), `NetworkInterface.getNetworkInterfaces`,
  `LinkProperties` (`getRoutes` / `getDnsServers` / `getHttpProxy`),
  `System.getProperty` for proxy keys, and redirects `/proc/net/*`
  reads done through `java.io.FileInputStream` / `FileReader` to
  `/dev/null`.
- The **vpnhide-zygisk** companion closes the native side:
  `libc::ioctl` (`SIOCGIFNAME` / `SIOCGIFFLAGS`) and `libc::getifaddrs`,
  which is what Flutter / Dart apps and any JNI code would hit
  bypassing ART entirely.

Split-tunnel is a hard requirement for the cards that compare the
device-reported public IP against external checkers: the detection
app's own HTTPS requests must exit through the carrier, otherwise the
checkers see the VPN exit IP and flag a mismatch with GeoIP / ASN
databases. That is a network-layer fact, not something any client-side
hook can fix.

### Source: official VPN/Proxy detection methodology

Both detection apps above (and any future ones playing the same game)
implement the **official Russian Ministry of Digital Development
methodology for identifying VPN/Proxy on user devices**, published as
an OCR'd Markdown copy here:
<https://t.me/ruitunion/893>. The Android sections (6.4 / 7.4 / 7.6 /
7.7) are the canonical reference for which Java APIs we hook and why.

### TODO — methodology coverage gaps

The methodology mentions a few additional vectors that we don't yet
handle. None of them are triggered by the two detection apps above on
a Pixel 8 Pro / Android 16, but they're documented signals and future
detectors will use them. Listed by descending priority:

- [ ] **`Runtime.exec("dumpsys vpn_management")` and `dumpsys activity
      services VpnService`** — sec. 7.4. On `untrusted_app` these
      always return `Permission Denial`, which both audited apps treat
      as a clean signal. Worth covering preemptively in case a future
      detector decides to interpret a non-empty stdout as positive
      proof.
- [ ] **`Runtime.exec("ip route" / "ip rule")` etc.** — same idea,
      same caveat (also denied for untrusted_app).
- [ ] **`NetworkScore` / `Score(Policies: IS_VPN)`** — sec. 6.4. Only
      reachable via reflection on system-internal API; normal apps
      can't read it. Theoretical leak. Defer until something actually
      tries.

The complementary native side (`getifaddrs`, `ioctl`, `/proc/net/*`
read by C/C++/JNI/Flutter that bypasses ART entirely) is the
responsibility of [vpnhide-zygisk](https://github.com/okhsunrog/vpnhide-zygisk),
not this module.

---

## What problem does this solve?

A lot of Android apps run an "is a VPN active?" check on startup (or before
sensitive actions like payments, account login, etc.) and refuse to work if
they detect one. They typically do this with a handful of very standard
Android APIs:

- Ask `ConnectivityManager` if any network has the `TRANSPORT_VPN` capability.
- Enumerate `NetworkInterface.getNetworkInterfaces()` and look for anything
  named `tun0`, `ppp0`, `wg0`, etc.
- Read `/proc/net/route` directly and look for a default route through a
  tunnel interface.
- Use the old `ConnectivityManager.getActiveNetworkInfo().getType() == TYPE_VPN`.

VpnHide intercepts all of these at the Java/Kotlin level (via LSPosed) so
that when a scoped app calls them, it gets back a picture of the world with
no VPN present. The VPN itself keeps working normally — this only affects
what the hooked app *sees*.

---

## What it hides

Every detection path below has a corresponding hook. If your target app only
uses these, VpnHide should make it blind to the VPN.

### 1. `android.net.NetworkCapabilities`
| Method | Behaviour with VpnHide |
|---|---|
| `hasTransport(TRANSPORT_VPN)` | always returns `false` |
| `hasCapability(NET_CAPABILITY_NOT_VPN)` | always returns `true` |
| `getTransportTypes()` | `TRANSPORT_VPN` stripped from the returned `int[]` |
| `getTransportInfo()` | returns `null` whenever the real value is `VpnTransportInfo` |
| `toString()` | post-processed: `\|VPN` stripped from `Transports:`, `VpnTransportInfo{…}` replaced with `null`, stray `IS_VPN` flags dropped from `&`-joined lists |

### 2. `android.net.NetworkInfo` (legacy `ConnectivityManager.getActiveNetworkInfo()` path)
| Method | Behaviour with VpnHide |
|---|---|
| `getType()` | returns `TYPE_WIFI` instead of `TYPE_VPN` |
| `getTypeName()` | returns `"WIFI"` instead of `"VPN"` |
| `getSubtypeName()` | any string containing "VPN" becomes empty |

### 3. `android.net.ConnectivityManager`
| Method | Behaviour with VpnHide |
|---|---|
| `getAllNetworks()` | VPN networks removed from the returned array |
| `getActiveNetwork()` | if the active one is a VPN, substitute the first non-VPN network |
| `getActiveNetworkInfo()` | if VPN, substitute the first non-VPN `NetworkInfo` |
| `getAllNetworkInfo()` | VPN entries removed |
| `getNetworkInfo(int type)` | returns `null` when asked about `TYPE_VPN` |
| `getNetworkInfo(Network)` | returns `null` if the result would be a VPN |

### 4. `android.net.LinkProperties`
| Method | Behaviour with VpnHide |
|---|---|
| `getInterfaceName()` | rewrites VPN interface names (`tun0`, `ppp0`, `wg0`, etc.) to `"wlan0"` |
| `getRoutes()` | routes whose interface is a VPN tunnel are dropped |

### 5. `java.net.NetworkInterface`
| Method | Behaviour with VpnHide |
|---|---|
| `getNetworkInterfaces()` | VPN tunnel interfaces removed from the enumeration |
| `getByName(name)` | returns `null` for names like `tun*`, `ppp*`, `tap*`, `wg*`, `ipsec*`, `xfrm*`, `utun*`, `l2tp*`, `gre*`, or anything containing `"vpn"` |
| `getByIndex(int)` | returns `null` if the looked-up interface is a VPN tunnel |
| `getByInetAddress(addr)` | returns `null` if the matched interface is a VPN tunnel |

### 6. `/proc/net/*` file reads
`FileInputStream` and `FileReader` constructors (both `String` and `File`
variants) are hooked. When an app tries to open any of the following paths,
the open is transparently redirected to `/dev/null`, so reads return EOF
immediately and the app sees no routes / no interfaces / no sockets:

```
/proc/net/route
/proc/net/ipv6_route
/proc/net/if_inet6
/proc/net/tcp
/proc/net/tcp6
/proc/net/udp
/proc/net/udp6
/proc/net/dev
/proc/net/arp
/proc/net/route_cache
/proc/net/rt_cache
/proc/net/fib_trie*
/proc/net/fib_triestat*
/proc/net/xfrm_stat*
```

### VPN interface name prefixes considered "a VPN"

`tun`, `ppp`, `tap`, `wg`, `ipsec`, `xfrm`, `utun`, `l2tp`, `gre`, plus
anything whose name contains the substring `vpn` (case-insensitive).

---

## What it does NOT cover (known gaps)

Be honest with yourself about what's here and what isn't. If a target app
does any of the things below, VpnHide won't be enough on its own.

### Native code path — not covered
Apps that enumerate interfaces or read `/proc` from C/C++ via JNI bypass
every Java hook in this module. The Linux syscalls that matter are:

- `getifaddrs()` / `freeifaddrs()`
- `ioctl(SIOCGIFCONF)` / `ioctl(SIOCGIFFLAGS)`
- raw `open("/proc/net/route", ...)` from libc, not java.io
- `sysconf(_SC_NPROCESSORS_ONLN)` is irrelevant, but you get the idea
- Direct `socket(AF_NETLINK, ...)` followed by `RTM_GETLINK` messages
  (modern native code tends to use this)

To intercept those you need a **Zygisk native module** that inline-hooks
libc. That's exactly what the [vpnhide-zygisk](https://github.com/okhsunrog/vpnhide-zygisk)
companion does — `libc::ioctl` and `libc::getifaddrs` patched in place
via ByteDance shadowhook. Install both modules together for full
coverage of the Java + native stacks.

### Server-side detection — unfixable client-side
No client-side module can fix any of this:

- **DNS leakage.** If the app resolves a hostname through the VPN resolver,
  the resolved IP comes from the VPN side and the server notices.
- **IP range blocklists.** Commercial "is this IP a known VPN exit?"
  databases (IPQS, MaxMind, IPHub, etc.) flag the source IP of your traffic.
  If your app talks to a service that uses them, they see the exit IP of
  your VPN provider and block you.
- **Latency fingerprinting.** Some backends measure RTT to known endpoints
  and notice the VPN hop.
- **TLS fingerprinting (JA3/JA4).** Completely orthogonal to VPN detection
  but worth knowing — the TLS handshake leaks a lot even through a tunnel.

The usual answer to all of these is **split tunnel**: make sure the target
app's traffic goes direct, not through the VPN, so the server only sees
your real ISP IP. VpnHide + split tunnel is the combo that tends to work.

### Process spawning — not covered
An app that does `Runtime.exec("cat /proc/net/route")` or
`ProcessBuilder.start()` to shell out and read `/proc` won't be caught by
the `FileInputStream` hook. Rare in practice but possible.

### `NetworkCallback` in-flight events — not deeply hooked
When an app calls `registerNetworkCallback()` and the system delivers an
`onCapabilitiesChanged(network, caps)` callback, the `caps` object it
receives still goes through our `hasTransport` hook — so checking
`caps.hasTransport(TRANSPORT_VPN)` in the callback returns `false`. But if
the app counts `onAvailable()` calls and infers VPN from "more than one
active network", that inference happens outside the method-level hooks.
Uncommon, but possible.

### VPN detection via `VpnService.prepare()`
`VpnService.prepare(Context)` returning an `Intent` doesn't necessarily mean
another VPN is active — it returns an `Intent` whenever the *calling* app
hasn't been granted VPN permission yet. Some detectors misread this, but
the module does not currently hook it. Add if needed.

---

## Threat model

VpnHide is designed for one specific scenario: *"I have a VPN running on my
phone and certain apps refuse to work because they detect it. I want those
specific apps to think the VPN isn't there, so I don't have to keep turning
the VPN off every time I use them."*

It is explicitly **not** designed for:

- Hiding root or custom ROM presence (that's a different problem — use
  Vector/LSPosed module scope, Tricky Store OSS, crDroid's built-in Play
  Integrity spoof, etc.)
- Bypassing Play Integrity's `MEETS_DEVICE_INTEGRITY` (unrelated — Play
  Integrity doesn't care whether a VPN is active)
- Fooling network-layer or server-side detection (client-side Java hooks
  can't do that — see "what it doesn't cover" above)

---

## Install

1. Build the APK (`./gradlew assembleDebug` → `app/build/outputs/apk/debug/app-debug.apk`).
2. Install it: `adb install app-debug.apk`.
3. Open your LSPosed / Vector manager, go to Modules, enable **VPN Hide**.
4. Add your target apps to the module's scope and **force-stop** them so
   they re-fork with hooks active (or reboot).
5. Open the **VPN Hide** launcher icon. You'll see a list of installed
   apps — tick the ones you want VPN hidden from. Use the overflow menu
   to toggle between "user apps only" and "show system apps".
6. Force-stop the target app again so it re-reads prefs on next launch.

### Double-gate logic

Two conditions must both be true for hooks to actually run inside an app:

1. The app is in the module's scope in LSPosed manager.
2. The app is checked in VPN Hide's picker UI.

Gate (1) controls which processes LSPosed loads the module into at all
(performance). Gate (2) is the per-app allowlist read at hook time via
`XSharedPreferences`. This lets you enable/disable hiding per-app without
going into the LSPosed manager every time.

---

## Debugging

```bash
adb logcat | grep VpnHide
```

On target app startup you should see:

```
VpnHide: installing hooks for com.example.targetapp
```

Any hook that fails to install is logged with the hook category and the
exception message, so you can tell whether a specific hook broke on a
newer Android version.

If hooks installed cleanly but the app still detects VPN:

```bash
# Grab a full trace around the detection event
adb logcat -c
# ... trigger the app's VPN check, then:
adb logcat -d > /tmp/detect.log
# Look for:
grep -iE "tun0|ppp0|wg0|vpn|TRANSPORT_VPN|NetworkInterface|/proc/net" /tmp/detect.log
```

Anything suspicious that isn't routed through VpnHide's filtered paths is
a clue about what the app is actually doing — e.g. a native library call,
a `ProcessBuilder.start("cat")` shell-out, or a server-side check.

---

## Build

Requires JDK 17 and Android SDK with `compileSdk = 35`.

```bash
cd vpnhide
gradle wrapper --gradle-version 8.9   # one-time, bootstraps ./gradlew
./gradlew assembleDebug
```

Output: `app/build/outputs/apk/debug/app-debug.apk`.

For a release build with proper signing, uncomment the `signingConfigs`
block in `app/build.gradle.kts` and provide a keystore.

---

## License

Personal / educational project. No explicit license — do whatever you want
with it but don't hold me responsible if a target app updates its detection
logic and breaks things.
