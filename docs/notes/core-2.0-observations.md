# Core observations and freshness (step 6)

## Model (`PROTOCOL_VERSION` 4)

`RoutingObservation`:

| Field | Meaning |
|-------|---------|
| `freshness` | `Never` / `Current` / `Unknown` |
| `generation` | Bumps on each publish |
| `vpn_interfaces` | Authoritative **only** when `Current` |
| `last_good_vpn_interfaces` | Display cache when `Unknown` — not current |
| `netlink_candidates` | Name-match ups (may include IMS `ipsec`) |
| `framework` | `TRANSPORT_VPN` channel status + ifaces |
| `uid` | Optional membership (`VPNHIDE_OBSERVE_UID`) |

Current requires **classified** framework evidence ∩ netlink ups. Netlink alone → Unknown.

`NativeBackendStatus`: `installed` (artifacts) ≠ `selected` (apply attempt) ≠ `applied` (Native Confirmed) ≠ `active` (runtime identity). Zygisk publication ≠ process loaded hooks.

## Sources

- Netlink link group (debounce 250ms; ENOBUFS → Unknown then resync)
- Framework: `VPNHIDE_FRAMEWORK_VPN_IFACES` stub, or `dumpsys connectivity` parse, or forced Unavailable/Failed via `VPNHIDE_FRAMEWORK_VPN`
- UID: `self_routed_for_interfaces` from `vpnhide_checks`

## Backend lifecycle refresh

Every ~30s lock-free `detect()` (kernel ctl + installed dirs). Does **not** take the KPM management lock (avoids racing Native apply). KPM active identity remains from startup/`detect_managed` / apply path.

## Device-only gaps

- Real `dumpsys` shape drift across Android versions
- Rule-only / split-routing edge cases beyond shared `self_routed_for_interfaces`
- Handover soak; live ENOBUFS under load
- App-context Zygisk hook checks (still APK-side)
