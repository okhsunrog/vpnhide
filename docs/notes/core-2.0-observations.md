# Core observations and freshness

Current implementation; app/core RPC protocol 11. See the
[current architecture](../core-architecture.md) for the complete ownership contract.

## Model

`RoutingObservation` distinguishes Never, Current and Unknown. Current VPN
interfaces require framework classification; last-good interfaces are display
history when the source is Unknown. Netlink name matches can include IMS and are
never sufficient proof of TRANSPORT_VPN.

UID evidence includes the observed main-app UID, Routed/Excluded/VpnOff/Unknown,
VPN session identity and interfaces. The UID is resolved from the same app data
directory that gates IPC, so reinstall updates membership without a daemon restart.
Session identity includes framework network IDs, preserving replacement of a VPN
that reuses an interface name. Unmanaged-tunnel membership retains its provenance.

`NativeBackendStatus` keeps installed artifacts, selected backend, applied policy
and live identity separate. `observed_freshness` qualifies the runtime read.
Zygisk publication is not proof that a process loaded its hooks. The boot decision
is core-owned and fixed per boot; observer refresh never replaces it or delivery.

## Sources and delivery

- Netlink link, IPv4/IPv6 route and rule notifications, debounced for 250 ms.
- Bounded dump resync at startup and after overflow.
- Two follow-up samples after an event (750 ms, then 1500 ms) catch delayed
  framework registration. A 30-second full fallback covers idle drift; app-dir UID
  metadata is checked every five seconds and changes trigger a full sample.
- Bounded framework classification via `dumpsys connectivity`, shared with explicit
  probe parsing; test overrides remain `VPNHIDE_FRAMEWORK_VPN_IFACES` and
  `VPNHIDE_FRAMEWORK_VPN`.
- UID membership via shared `self_routed_for_interfaces`.
- Thirty-second backend observation refresh runs outside the async executor;
  KPM reads use the management lock. A boot/delivery change supersedes an older
  in-flight observation before publication.

An interrupted/failed event source withdraws current evidence and is reopened.
Unchanged reconciliation does not bump routing generation. Core publishes changes
through state_since; the app uses one persistent CoreSession long-poll, not a
foreground app-VPN polling timer. The app rejects stale epochs and treats a
transport disconnect as unavailable current evidence.

## Validation boundary

Host tests cover link/rule invalidation, dump deletion/interruption/sequence,
unchanged samples, session identity, unknown/last-good composition and client
restart/disconnect. Real dumpsys shape drift, Android UID rules, SELinux, handover,
ENOBUFS under device load and app-context hooks still need device validation.
