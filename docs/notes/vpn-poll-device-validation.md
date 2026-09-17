# Foreground VPN polling and callback lifecycle validation

2026-09-17, `refactor/ui-situation`, based on `54bd755e` plus the working-tree
polling/callback changes. This is device acceptance evidence, not a CI result.

## Build and device

- Pixel 8 Pro, Android SDK 37, built-in Native backend and LSPosed active.
- Installed minified debug APK: `1.2.5-253-g54bd755e-dirty`.
- APK SHA-256: `ff6232626dd562c197cbb3378b97b9637164cc543787e14eee752e365bbc19d2`.
  Installed `base.apk` matched the local build hash.
- Rebooted after install; the hook report showed the matching dirty version and
  current boot ID, with both callback overloads attached and zero attach errors.
- 800 JVM tests passed, together with warnings-as-errors compilation, detekt,
  CPD, Android lint, ktlint, shellcheck and debug APK assembly.

## App refresh

Kept the Dashboard foreground and controlled Floppa VPN through its ADB receiver.
Agent `getState(refresh=false)` and a continuous logcat capture were used; no
Retry or forced agent refresh was used to cause the transitions.

- VPN off -> on: protection changed from `VPN_OFF` to `ROUTED` automatically.
- VPN on -> off: protection returned to `VPN_OFF` automatically.
- Stable polls did not repeatedly start full gate/root refreshes.
- HOME stopped polling after the current worker finished. No new poll appeared
  between 02:09:52 and foreground return at 02:10:33. VPN was enabled while the
  app was backgrounded; foreground return refreshed it to `ROUTED` without Retry.
- Final screenshot showed "VPN hidden", Native/Java OK, three modules active,
  and zero issues.
- 39 observed lightweight samples: 106–627 ms, median 344 ms. These are durations
  on this device, not energy measurements or guarantees for other ROMs. The
  four-second delay is between completed polling iterations, not fixed-rate work.
  (The interval was subsequently set to one second at the maintainer's request;
  that changes how often a sample runs, not the per-sample cost measured above.)

## Callback captures

Used `scripts/network-view-probe.py` under VPN Hide's target UID. App-process
diagnostics separately passed the pushed-capabilities/link-properties check.

- Unchanged Wi-Fi, VPN on and off within a 20-second capture: default registration
  received only its initial Available/Capabilities/LinkProperties/Blocked bundle.
  No repeated lifecycle or property events; VPN-specific listener stayed silent.
- Two concurrent probe processes under the same target UID each retained an
  independent default registration. Both recorded Wi-Fi 106 -> cellular 101 ->
  Wi-Fi 107, with initial properties for every replacement and no VPN handle.
- Airplane mode plus Wi-Fi disabled produced one `Lost(107)`; restoring airplane
  mode off and Wi-Fi on produced `Available(108)` and its initial properties.
- Dynamic captures explicitly reported topology changes during collection. Their
  synchronous before/after snapshots are not treated as stable invariant proofs.
- The final stable VPN-on capture had no errors or violated invariants: active
  handle/list/properties agreed, no phantom networks or VPN transports, legacy
  VPN type inactive, VPN listener silent. PendingIntent was not captured.
- No callback-adapter failure or fatal exception appeared in the captured logs.

Temporary offline does not necessarily mean the VPN tunnel is down: Floppa kept
its VPN active during the airplane test. The offline assertion concerned loss
and recovery of the app-visible cover network, not a forced `VPN_OFF` gate.

## Boundaries and restoration

This run covers one Android 17 ROM and the built-in backend. It does not establish
behavior on Android 9–12/non-Bundle dispatchers, OEM callback variants, other root
managers, or PendingIntent registrations. Two same-UID probe processes are not a
test of every possible within-process registration pattern.

Airplane mode was restored to off, Wi-Fi and mobile data to on, VPN left connected.
The logcat recording was stopped. Local evidence files are in `/tmp` with prefix
`vpnhide-` (state JSON, callback captures, dashboard screenshot and device log);
they are temporary artifacts and are not committed user data.
