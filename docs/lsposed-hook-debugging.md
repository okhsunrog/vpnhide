# Debugging the LSPosed (Java-layer) hooks

How to tell whether the LSPosed module's `system_server` hooks actually
attached, how to read the install telemetry, and how to diagnose "some apps
still detect my VPN" reports that come down to a hook not attaching.

This is the **Java-layer** counterpart to [diagnostics.md](diagnostics.md) (the
self-test model), [detection-vectors.md](detection-vectors.md) (which backend
covers which vector), and [lsposed/AGENTS.md](../lsposed/AGENTS.md) (the Kotlin
module architecture). The wire the state file speaks is in
[protocol.md](protocol.md).

The hooks live in `lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/hook/HookEntry.kt`;
the state file they publish is written by `LsposedState.kt` and rendered by
`HookDiagnostics.kt`.

## 1. Two hook families (why some checks pass while others leak)

The module installs two very different kinds of hook inside `system_server`, and
they fail independently — so a half-broken Java layer is normal to see:

- **`writeToParcel` sanitizers** on `NetworkCapabilities`, `LinkProperties`,
  `NetworkInfo`. These are **framework classes on the boot classpath**, always
  resolvable, so they attach trivially and rarely break. They strip VPN data as
  the object is serialized to a target UID.
- **`ConnectivityService` method hooks** — hooks on the actual service methods
  (`getActiveNetwork`, `getAllNetworks`, `getNetworkForType`, `getNetworkInfo`,
  the callback dispatchers). These replace/blank the **network handle identity**
  and legacy-type answers. On Android 13+ the class lives in the Connectivity
  **APEX**, so these can only be hooked via the service binder's classloader —
  this is the fragile family (see §8).

Which diagnostics check leans on which family:

| Check | Family | Notes |
|---|---|---|
| hasTransport(VPN), hasCapability(NOT_VPN), getTransportInfo | 1 | capabilities |
| getAllNetworks() VPN scan | 1 | capabilities of each net |
| LinkProperties ifname / routes | 1 | link properties |
| ActiveNetwork **transports** | 1 | capabilities of the active net |
| getNetworkForType(TYPE_VPN) | 2 | legacy type → handle |
| ActiveNetwork **handle** | 2 | netId identity |
| getAllNetworks() **handles** | 2 | netId identity |
| NetworkCallback (push) | 2 | callback dispatch |
| getNetworkInfo(TYPE_VPN) | 2 | legacy type query |

The tell-tale signature of "family 2 didn't attach" is that **ActiveNetwork
transports passes but ActiveNetwork handle fails on the same active network**:
the returned handle is still the VPN's netId, yet its capabilities come back
clean. Family 1 sanitized the capabilities; family 2 never swapped the handle.

## 2. Where the truth lives: `/data/system/vpnhide_lsposed_state`

The module publishes a small text "control channel" from `system_server`; the
app reads it. It survives reboots and is **independent of logcat and the
debug-logging toggle**. Three sections:

```
vpnhide 1 status        # backend id, kernel ver, hooks bitmask, error code
backend 0x3
kver 0x0
hooks 0x3fc00
error 0x0
meta <key> <value>      # one line per metadata key
...
vpnhide 1 stats         # per-uid per-hook fire counters
0x27fe 0xa:0x1 0xb:0x1 0xd:0x1 0xe:0x1 0x10:0xc
```

It reaches a debug bundle **two ways** (the bundle is a single `state.json`
packed in the export zip; the state file is read as root into the snapshot):

- the **`hookReport`** field — parsed and rendered (status mask, `metadata:`
  block, installed/missing hooks, counter deltas). This is what you normally read.
- the raw file verbatim, in the **`sections.lsposed_state`** entry.

Because the read is root-backed and the file is always written, `cs_*` telemetry
lands in the bundle **even with debug logging off**. Debug logging only enriches
the *logcat* portions of the bundle.

## 3. Reading the `hookReport` field (the LSPOSED section)

```
LSPOSED:
  status.hooks=0x3fc00
  status.error=OK(0)
  installed hooks (8/8):
    [10] lsposed_link_properties ...
  metadata:
    cs_path=C
    cs_network=getActiveNetwork=1,getAllNetworks=1,getNetworkForType=1
    ...
```

The mask is **honest**: the three connectivity bits are set only once those
hooks actually attach (see §8). A device where they don't shows
`status.error=PARTIAL_HOOKS`, `installed hooks (5/8)`, and lists them under
**missing owned hooks** — not a false "8/8 installed".

Hook IDs (bit = id):

| id | hex | name | family | covers |
|---|---|---|---|---|
| 10 | 0x400 | lsposed_link_properties | 1 | ifname / routes / DNS |
| 11 | 0x800 | lsposed_network_capabilities | 1 | transports / NOT_VPN |
| 12 | 0x1000 | lsposed_network_info | 1 | legacy NetworkInfo type |
| 13 | 0x2000 | lsposed_network | 2* | Network parcel replacement |
| 14 | 0x4000 | lsposed_connectivity_result | 2 | getNetworkInfo/LP/NC results |
| 15 | 0x8000 | lsposed_connectivity_callback | 2 | push callbacks |
| 16 | 0x10000 | lsposed_connectivity_network | 2 | getActiveNetwork/AllNetworks/ForType |
| 17 | 0x20000 | lsposed_package_visibility | — | app-hiding from PackageManager |

`0x3fc00` = all of 10–17. The connectivity family is 14|15|16 = `0x1c000`.
(*13 depends on the CS instance being captured, so it dies with family 2.)

The `stats` counters show which hooks actually **fired** for the target app's
uid. On a healthy device you see `0xe` (result), `0x10` (network), `0xd`
(Network parcel) among them; their absence is the first hint family 2 is down.

## 4. The `cs_*` attach telemetry

`ConnectivityService` attaches **asynchronously and by several paths**, so the
outcome is recorded into `cs_*` meta keys as it happens:

| key | meaning |
|---|---|
| `cs_attempts` | full trail: `install … \| A:… \| B:… \| C:… \| D:…` — every path's outcome |
| `cs_path` | which path finally attached (`A`/`B`/`C`/`D`); **absent if none did** |
| `cs_class` | resolved class (`com.android.server.ConnectivityService`) |
| `cs_loader` | classloader chain, e.g. `PathClassLoader<PathClassLoader<BootClassLoader` (double = APEX) vs single `PathClassLoader<BootClassLoader` (system_server) |
| `cs_ctor` | constructors hooked |
| `cs_result` | per-method match counts for synchronous results (`getNetworkCapabilities`, `getNetworkInfo`, …) |
| `cs_network` | `getActiveNetwork` / `getAllNetworks` / `getNetworkForType` counts |
| `cs_callback` | `callCallbackForRequest` / `sendPendingIntentForRequest` counts |

**Known-good fingerprint (Android 13+, attach via the binder classloader):**

```
cs_path      C            (or D)
cs_loader    PathClassLoader<PathClassLoader<BootClassLoader
cs_network   getActiveNetwork=1,getAllNetworks=1,getNetworkForType=1
cs_callback  callCallbackForRequest=1,sendPendingIntentForRequest=1
```

`A:notReady(ClassNotFound com.android.server.ConnectivityService)` in the trail
is **normal** on A13+ — path A can't see the APEX class, so it hands off to the
binder-classloader paths.

## 5. Interpreting a failing report

Read `cs_attempts` top to bottom and match the divergence from §4:

| Symptom in `cs_*` | Meaning |
|---|---|
| `cs_path` absent; trail ends `… \| B:getService=null` with **no `C`/`D` attach** | Hooks never attached — the service was never resolved. |
| Trail has **no `C:addService(connectivity) seen`** | The ROM doesn't publish `connectivity` through the hooked `ServiceManager.addService` (seen on MediaTek/OEM). Path D (deferred `getService`) is the fallback that covers this. |
| `cs_network` has `getNetworkForType=0` (etc.) | That method name/signature differs on the ROM; `hookAllMethods` matched nothing. |
| `cs_path` set, `cs_network` all `=1`, mask full, **but the connectivity `stats` counters never move** and checks leak | Hooks bound but never fire. Two known causes, both fixed (§8): an early pre-construction attach ART discarded (the install-time classloader path, removed), or a name-resolved class that isn't the live instance — look for `nameResolvesSame=false` in `cs_attempts` (now we hook `binder.javaClass` directly). |

## 6. Why logcat is not the tool here

The install runs at early boot with debug logging **off** (the flag is read that
early and defaults false), so the `HookLog.i` install lines are never emitted;
and even when emitted they are gone by report time (ring-buffer rotation). This
was confirmed on a *working* device: debug on, zero install lines in logcat, yet
hooks attached. Always use the state file / the `hookReport` field, not logcat, to
answer "did the hooks attach".

## 7. Collecting a report (users / testers)

Prerequisites:

- Root granted to the app (the state file is read via root).
- The VpnHide module **enabled with "System Framework" scope** in your Xposed
  manager (it hooks `system_server`, nothing else).
- An **active VPN** — the leak checks only run against a live tunnel.

Steps:

1. Reboot (so a fresh attach writes current `cs_*` into the state file).
2. Open VPN Hide once with the VPN connected (its checks run at cold start).
3. Diagnostics → **Collect debug logs** → send the zip.

Debug logging does **not** need to be on for `cs_*` — it only adds the logcat
sections. Open the bundle's `hookReport` field and read §3–§5.

## 8. The attach mechanism (developers)

`HookEntry.installConnectivityServiceHook` only ever attaches from the **live
service binder**, never via the raw `system_server` classloader. A classloader
attach at install time runs *before* `ConnectivityService` is constructed; ART
then replaces the method entries during class init/compile and the hooks
silently never fire — observed on MediaTek A11, where every method matched and
the mask read full yet no connectivity counter ever moved. It tries, in order:

- **B** — `getService("connectivity")` now, hook from its binder's classloader.
  Usually `null` at install time (service not registered yet).
- **C** — hook `ServiceManager.addService` to catch the registration and take the
  binder's classloader. Fast when it fires — but only fires if the ROM actually
  publishes through that Java method.
- **D** — deferred fallback: poll `getService("connectivity")` on a short-lived
  `HandlerThread` (500 ms, ~90 s budget) until the live binder appears, then
  attach. Independent of *how* the service was registered; this is what covers
  ROMs where C never fires. The thread is torn down (`quitSafely`) the moment any
  path attaches or the budget runs out.

All three attach *late*, to an already-constructed instance — which is why the
hooks stick (an install-time classloader attach does not).

Two facts make B/C/D work:

1. Inside `system_server`, `getService`/the `addService` argument return the
   **local `ConnectivityService` instance**, not a `BinderProxy`, so
   `binder.javaClass` is the live class.
2. We hook **`binder.javaClass` directly**, never a name-resolved class. Resolving
   `com.android.server.ConnectivityService` by name through the binder's
   classloader can, on a child-loader ROM (MediaTek A11), follow delegation to a
   *parent* copy of the class — a different `Class` object with the same name.
   Hooking that copy attaches cleanly (all methods match, mask full) but never
   fires, because the live binder dispatches to the child copy. The
   `nameResolvesSame` flag in `cs_attempts` records whether name-resolution would
   have returned that same live class (`true` on Pixels; `false` is the trap).

The bits and `cs_*` are reported from `reportConnectivityAttach` →
`LsposedStats.setConnectivityDiagnostics`, which folds the bits into the mask and
the meta into the state file. To add a new signal, record it there — every
`meta` key is rendered by `HookDiagnostics.kt` automatically, no reader change
needed.

> Note on module state after updating the APK: normally an Xposed manager keeps a
> module enabled (with its scope) across an in-place update. If after installing a
> new build the module shows disabled or unscoped, that is a manager/device
> quirk — just re-enable it with **System Framework** scope; it is not expected
> behavior and nothing in VpnHide changes it.
