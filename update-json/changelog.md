## v1.3.0

### Added
- A built-in (in-tree, CONFIG_VPNHIDE=y) kernel backend and its companion module for kernels the loadable .ko cannot run on: integrated-modules, no-kprobes or whole-program-LTO kernels (e.g. Sultan). Same hiding coverage as the .ko, compiled into the kernel. The app recognises the built-in backend, picks its matching companion for activation, explains a redundant or missing companion, and refuses to load or configure the .ko over a live built-in backend.
- armeabi-v7a (32-bit ARM) support: the app and the Zygisk module now install and run on 32-bit ARM devices.
- In-app offline help in English, Russian and Chinese: a searchable guide opened from Settings, the Hiding tab and the Dashboard, including "Learn more" links on dashboard issue cards. It replaces the long inline help block on the Hiding screen and covers backend setup, saving and applying changes, what the diagnostics can and cannot tell, debug reports, profiles, backups and removal; the first article answers the question the roles list left open: configure the app you are hiding the VPN from, not the VPN app.

### Changed
- Diagnostics now explain a partially loaded native backend instead of leaving a red leak unexplained: the dashboard says how many kernel hooks installed and which ones the kernel did not expose, and the leaking check names the hook that is missing. A KPM whose control call is refused while no SuperKey is saved now says so, instead of suggesting a reinstall that cannot help.
- Bug-report bundles and the agent bridge now include the same diagnostic summary the app shows: whether the last check still applies, whether it was interrupted, and why a check cannot run right now. Collecting a debug log also no longer fails silently when the app cannot tell whether its own traffic goes through the VPN: the archive is written either way and records how the self-test ended and which run the results came from.
- Clearer diagnostics message when VPN Hide isn't routed through the active VPN: some VPNs route only selected apps and won't let you add VPN Hide, which is expected rather than a failure
- The dashboard appears about twice as fast on a cold start (roughly 3 s to 1.5 s on a Pixel 8 Pro): the app reads the root state once instead of two or three times, the first root shell doubles as the root check, the privileged config session opens in one round trip, and the startup runtime re-activation runs after the first screen is shown.
- The Dashboard status and the detailed diagnostics now say what the results mean and show the same state. While the VPN state or the hiding checks are being re-checked, the status card shows a neutral "Checking…" with a progress indicator instead of claiming "VPN hidden" or flashing "Couldn't determine whether VPN Hide is routed through the VPN" on a VPN toggle; a background re-read that finishes quickly changes nothing on screen. When the status is not simply green, the card says why: a configuration change still applying, unknown VPN routing, an interrupted or failed check, results that predate a VPN or configuration change, a check that measured nothing, or a needed restart. A failed network read is reported as unverified rather than as a leak, and a blocked or failed check is no longer restarted over and over by cache refreshes.
- The self-test now waits for a pending save that affects VPN Hide's own hiding and re-runs after it lands, instead of measuring while the change is half applied; saves for other apps no longer touch it.
- Saving in the Hiding tab no longer shows a "Saved N target(s)" snackbar over the navigation bar; the settled switches and the disabled Save button are the confirmation. Errors and the native capacity warning still appear.
- The Dashboard no longer animates the protected shield icon. The looping scale was barely visible, but it kept the screen redrawing at the display's refresh rate and the app on a full CPU core for as long as the Dashboard was open.

### Fixed
- Settings from versions before 1.0 are imported again. Upgrading straight from 0.7.x to 1.2.x left the old lists on disk unused, so the app looked reset. They are now folded into the current config automatically when nothing is configured yet; when something already is, the dashboard offers to merge them, replace the current list with them, or delete them without importing. Either way the old files finally get cleaned off the device.
- The diagnostic bundle collects its forensic sections again. In 1.2.5 an apostrophe inside the root probe script left a quote unbalanced, so the whole batched command failed to parse and bundles came back without the network, module and app-scan sections.
- KPM now finds hook targets on kernels that rename them (Clang CFI with full LTO, seen on MediaTek 4.14). Three hooks silently failed to install there, which left ioctl(SIOCGIFCONF) enumerating the VPN interface for target apps.
- A failing native probe no longer takes the app down with it. The probe library now unwinds instead of aborting, so an unexpected kernel reply surfaces as one failed check run — with the panic message and its source line in logcat — instead of killing the process at startup.
- Changing a setting no longer risks dropping an app's hiding roles. Toggling the SuperKey or experimental protection rebuilt the whole app list from a projection that resolved app-hiding targets through the installed-app list, so an app in a profile the scan could not read lost its role on save.
- The bind-to-interface vector is covered again on kernels below 5.9. The kernel backends only hooked the resolved-ifindex helper on 5.7–5.8 and otherwise relied on the kernel refusing an unprivileged bind, which a LineageOS 5.4 build did not; the helper is now found by symbol, and when a kernel exposes neither, the backend no longer claims to cover the vector. The Zygisk backend likewise decided from the kernel version and stayed silent where that assumption was wrong; it now measures what the running kernel answers for an interface that does not exist and gives a hidden one exactly the same answer.
- The Zygisk backend no longer risks crashing a target app that passes a bad pointer to an interface ioctl. The interface name is now read through the same fault-contained path the setsockopt hook uses, so such a call gets the kernel's own EFAULT instead of a segfault inside the app.
- Debug bundles from phones with very many apps are complete again. The per-user app scan could use up the whole time budget, and everything after it — the network and routing sections a connectivity report actually needs — was cut off. Those sections are now collected first, and the budget is larger.
- Turning Debug logging off now takes effect in the LSPosed hooks without a reboot. The flag was refreshed only by a filesystem watcher, and when that watcher stopped delivering events the hooks kept writing to logcat for as long as the device stayed up.
- Distinguish carrier IMS tunnels from active user VPNs in diagnostics, and check root-tunnel routing without changing interface hiding.
- Configuration switches now show the requested value and progress immediately, without moving the screen while saving; errors appear as overlays with recovery actions. App edits survive Activity recreation, concurrent writes preserve unrelated fields, and root operations with an unknown outcome pause safely after one automatic recheck.
- The debug-logging banner now points to Settings -> Developer, where the toggle actually is (it said Debugging)
- Detailed diagnostics keep running when you leave the screen or rotate the device, and a retry always starts a fresh run instead of reusing an old failure.
- Keep the current tab, nested settings screen, search and filters when Android recreates the activity after rotation or a system theme change.
- The startup loading skeleton now sits inside the real screen with its header and tab bar, so the layout no longer shifts down once the dashboard has loaded.
- Status banners, icon bubbles and the system-bar icons now follow the in-app theme, so a manual Dark or Light choice no longer drifts to the wrong colors when the system theme differs
- Agent control can no longer make VPN Hide itself a target of the ports module. That setting rejected the app's own localhost connections, which cut the agent bridge off until the change was reverted by hand, and could crash the app when the bridge's client disconnected mid-request. VPN Hide's own roles are now fixed on every write path, and a client disconnecting mid-request is handled.
- Error messages after a failed save now appear above the Save / Discard bar and the tab navigation instead of covering them.
- Legacy VPN network queries now preserve the disconnected VPN type instead of returning null or renaming it to Wi-Fi, avoiding false detection by apps such as older Ruru releases. Diagnostics now check the returned legacy network type and its enumeration as well as connection state; a null reply is reported as inconclusive instead of a successful check.
- Target apps now see a single, consistent network model while the VPN is hidden: the network handle, its capabilities, link properties and legacy NetworkInfo, plus push callbacks and PendingIntents, all describe the physical network behind the VPN, matching what the app sees with no VPN and closing detection that cross-checked these answers against each other. Callback registrations keep their own visible network lifecycle without duplicate events across VPN transitions, and the cover network is chosen with the legacy per-UID blocking policy taken into account.
- The Dashboard and Diagnostics now follow VPN changes reliably. Turning the VPN on or off is noticed without a manual refresh, and re-adding VPN Hide to a tunnel (or a tunnel re-established) while the app was in the background re-checks hiding on its own when you return, instead of stopping at "Re-check needed" or flashing a re-check state; the Dashboard tiles follow that check. The app no longer depends on network callbacks for this, which its own Java hook hides from it: while it is open it observes its own VPN state directly.
- The Dashboard could show the Ports module as inactive, with a warning, after a root snapshot raced another iptables user (netd on a network change, the VPN client). The probe now waits for the xtables lock, and a probe that still could not run shows the module as not verified instead of inactive.

## v1.2.5

### Added
- Warn before collecting a debug log or logcat recording while the VPN is off or VPN Hide is not routed through the tunnel — such captures are incomplete and hard to diagnose. Offers to re-check or proceed anyway.

### Changed
- The VPN-tunnel check is now shared across the whole app — re-checking in one place updates every screen, and Diagnostics now reacts automatically when you turn the VPN on or off.

### Fixed
- Fixed the kernel module failing to load on some OEM kernels with a trimmed module symbol table (e.g. Xiaomi HyperOS android12-5.10), where dev_get_by_index_rcu was reported as an unknown symbol; it is now resolved at runtime via kallsyms like path_put, so the module links without a hard reference.
- Settings switches no longer flicker off/on while a debug log is being collected, and the debug-export sheet no longer keeps a stale collected file when reopened.
- The app list now loads reliably on devices with many installed apps and/or multiple profiles (common on MIUI/HyperOS). It no longer fails on huge app counts or blocks the whole list demanding you unlock a profile — it shows what it can read and flags any profile it couldn't; settings for un-scanned profiles are preserved.

## v1.2.4

### Changed
- The debug export now bundles a single self-contained diagnostics file (state.json) in the .zip, replacing the old pile of separate text files — one file has everything.

### Fixed
- A hiding module could show a false "inactive" on some KernelSU devices when the status probe couldn't read the module's liveness; it now reads "status not verified" instead of claiming the module is off.
- KPM: on some kernels built with Clang LTO the IPv6 `/proc/net/ipv6_route` hook could silently fail to install, leaving native hiding partial. It now installs.
- SO_BINDTODEVICE hiding could fail on GKI kernels where the compiler dropped setsockopt's unused level argument, letting a target app still bind a socket to the VPN interface

## v1.2.3

### Fixed
- The app list no longer fails to load with a "couldn't read all Android profiles" error when a profile is legitimately empty — a scan that succeeds (exit 0) with no packages is now accepted, and only a profile whose scan actually errors blocks the list (seen on a Motorola vendor profile that reports zero apps).

## v1.2.2

### Fixed
- KPM: harden the route and interface hooks against vendor kernels whose struct layout differs from the built-in offset table — a mismatched offset now degrades to "not hidden" instead of risking an out-of-bounds skb write or a dereference of a bogus device pointer (reported as a spontaneous reboot on a vendor 5.4 kernel).
