# vpnhide -- LSPosed module

Hooks Java network APIs to hide VPN presence from selected apps. Part of [vpnhide](../README.md).

## Modes

1. **App-process mode** (default) -- hooks installed directly in target app processes via LSPosed/Xposed. More complete coverage (all APIs below) but visible to anti-tamper SDKs that scan for Xposed artifacts.
2. **system_server mode** -- hooks `writeToParcel()` in `system_server` so VPN data is stripped before Binder serialization. Zero presence in the app process. Use with [kmod](../kmod/) for native coverage.

---

## What it hooks (app-process mode)

### 1. `android.net.NetworkCapabilities`
| Method | Behaviour with VpnHide |
|---|---|
| `hasTransport(TRANSPORT_VPN)` | always returns `false` |
| `hasCapability(NET_CAPABILITY_NOT_VPN)` | always returns `true` |
| `getTransportTypes()` | `TRANSPORT_VPN` stripped from the returned `int[]` |
| `getTransportInfo()` | returns `null` whenever the real value is `VpnTransportInfo` |
| `toString()` | post-processed: `\|VPN` stripped from `Transports:`, `VpnTransportInfo{...}` replaced with `null`, stray `IS_VPN` flags dropped from `&`-joined lists. Uses string manipulation (not regex) to avoid `PatternSyntaxException` on edge cases. |

### 2. `android.net.NetworkInfo`
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
| `getDnsServers()` | returns empty list for VPN LinkProperties |
| `getHttpProxy()` | returns `null` for VPN LinkProperties |

### 5. `java.net.NetworkInterface`
| Method | Behaviour with VpnHide |
|---|---|
| `getNetworkInterfaces()` | VPN tunnel interfaces removed from the enumeration |
| `getByName(name)` | returns `null` for VPN-like names |
| `getByIndex(int)` | returns `null` if the looked-up interface is a VPN tunnel |
| `getByInetAddress(addr)` | returns `null` if the matched interface is a VPN tunnel |

### 6. `/proc/net/*` file reads
`FileInputStream` and `FileReader` constructors (both `String` and `File` variants) are hooked. Opens to the following paths are redirected to `/dev/null`:

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

### 7. `System.getProperty`

Proxy-related system properties that can leak VPN presence:

| Key | Behaviour with VpnHide |
|---|---|
| `http.proxyHost` | returns `null` |
| `http.proxyPort` | returns `null` |
| `https.proxyHost` | returns `null` |
| `https.proxyPort` | returns `null` |
| `socksProxyHost` | returns `null` |
| `socksProxyPort` | returns `null` |

### VPN interface name prefixes

`tun`, `ppp`, `tap`, `wg`, `ipsec`, `xfrm`, `utun`, `l2tp`, `gre`, plus anything whose name contains the substring `vpn` (case-insensitive).

---

## system_server mode

### When to use

For apps with aggressive anti-tamper SDKs that detect app-process hooks (crashes, NFC payment degradation, etc.). The default app-process hooks cannot be used for these apps.

### How to enable

1. In LSPosed/Vector manager, add **"System Framework"** to this module's scope.
2. Reboot so the module loads into `system_server`.
3. Install [vpnhide-kmod](../kmod/) for native-side coverage.
4. Manage targets via kmod's WebUI, which writes UIDs to `/data/system/vpnhide_uids.txt`.

### What it hooks

`writeToParcel()` on `NetworkCapabilities`, `NetworkInfo`, and `LinkProperties`. Uses a ThreadLocal save/restore pattern so the original values are preserved for non-target callers. Per-UID filtering via `Binder.getCallingUid()` ensures only target apps see the filtered view.

### Target management

A `FileObserver` (inotify) watches `/data/system/` and reloads the UID list immediately when the file changes. No reboot needed.

**Important:** apps with aggressive anti-tamper SDKs must NOT be added to this module's LSPosed app-process scope. Only "System Framework" should be in scope for these apps.

---

## Install

1. Build the APK (`./gradlew assembleDebug`).
2. Install: `adb install app/build/outputs/apk/debug/app-debug.apk`.
3. Open LSPosed/Vector manager, go to Modules, enable **VPN Hide**.
4. Add target apps to the module's scope and **force-stop** them (or reboot).
5. Open the **VPN Hide** launcher icon. Tick the apps you want VPN hidden from.
6. Force-stop the target app again so it re-reads prefs on next launch.

### Double-gate logic

Two conditions must both be true for hooks to run inside an app:

1. The app is in the module's scope in LSPosed manager.
2. The app is checked in VPN Hide's picker UI.

Gate (1) controls which processes LSPosed loads the module into (performance). Gate (2) is the per-app allowlist read at hook time via `XSharedPreferences`.

---

## What it does NOT cover

- **Native code path** -- apps checking VPN from C/C++/JNI/Flutter bypass all Java hooks. Use [zygisk](../zygisk/) or [kmod](../kmod/).
- **Server-side detection** -- DNS leakage, IP blocklists, latency/TLS fingerprinting. Unfixable client-side; use split tunneling.
- **`Runtime.exec` / `ProcessBuilder` shell-outs** -- e.g. `cat /proc/net/route` in a subprocess.
- **`NetworkCallback` event counting** -- inferring VPN from `onAvailable()` call count.
- **`VpnService.prepare()`** -- not currently hooked.

See [the project README](../README.md) for the full threat model and split-tunnel requirements.

---

## Debugging

```bash
adb logcat | grep VpnHide
```

On target app startup you should see:

```
VpnHide: installing hooks for com.example.targetapp
```

Any hook that fails to install is logged with the hook category and exception message.

If hooks installed but the app still detects VPN:

```bash
adb logcat -c
# trigger the app's VPN check, then:
adb logcat -d > /tmp/detect.log
grep -iE "tun0|ppp0|wg0|vpn|TRANSPORT_VPN|NetworkInterface|/proc/net" /tmp/detect.log
```

---

## Build

```bash
./gradlew assembleDebug
```

Requires JDK 17. Output: `app/build/outputs/apk/debug/app-debug.apk`.

---

## License

Personal / educational project. No explicit license -- do whatever you want with it but don't hold me responsible if a target app updates its detection logic and breaks things.
