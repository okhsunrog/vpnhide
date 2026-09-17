# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

## v1.2.1

### Changed
- The KPM (KernelPatch Module) backend is now considered stable — its experimental/beta labels and the "if something breaks, fall back" warning cards have been removed.

### Fixed
- A freshly installed native module (kmod/KPM) that is only waiting for its first reboot now shows "Installed, reboot needed" instead of a misleading "installation corrupted — reinstall" error.
- Kernel module (.ko) no longer fails to load on OEM kernels that don't export path_put (e.g. some OnePlus android14-6.1 builds), where the backend showed as installed but inactive with a "missing vpnhide_kmod.ko" error.
- Port-hiding activation no longer fails when another process (netd, an OEM firewall) briefly holds the iptables lock — the activator now waits for the lock instead of aborting the whole apply.
- When the KPM backend is installed but the kernel has no live KernelPatch runtime to load it (detected via `kpatch hello`, the same check KPatch-Next uses), the dashboard now tells you to install the KPatch-Next-Module and patch your kernel from its interface — or use APatch/FolkPatch — instead of a cryptic "kpatch CLI not found — reinstall the zip" error.

## v1.2.0

### Added
- Selected apps can no longer bind sockets to hidden VPN interfaces through SO_BINDTODEVICE or SO_BINDTOIFINDEX on the kernel backends, with a best-effort libc fallback for Zygisk-only installs.
- KPM supports Android 4.9 kernels, with the matching in-app installation recommendation; legacy 4.14/4.19 validation moved to pinned AOSP common reference kernels.
- Zygisk now hides VPN routing policy rules (ip rule / RTM_GETRULE) from target apps — a detection vector that previously only the kernel backends (.ko / KPM) covered, so Zygisk-only devices leaked it.
- Simplified Chinese (zh-rCN) app translation and a Chinese README.
- The Dashboard shows a one-time support request once hiding is confirmed working, with Support and Hide.
- The minimum supported Android version is now 9 (API 28).
- Optional filesystem path hiding for VPN interfaces under sysfs and /proc/sys/net, on all three native backends. It is an explicit opt-in: the UI warns about the possible slowdown and shows whether the reboot has taken effect. kmod and KPM apply it after a reboot and install no VFS hooks at all while it is off; Zygisk covers it best-effort in-process, filtering both libc readdir and raw getdents64 so native and Rust callers cannot enumerate those directories either.
- Settings has a "Support the project" entry with Boosty and crypto donation options.

### Changed
- The GKI kernel module stays loaded until reboot, matching the Magisk, KernelSU, and APatch module lifecycle.
- Detailed diagnostics headlines how many vectors are hidden vs leaking instead of a misleading passed-count.
- Diagnostics debug export now records each check's full outcome (leak / hidden by backend / hidden by SELinux) and the root ground-truth behind it, plus each layer's verdict and a machine-readable diagnostics.json — instead of a plain pass/fail list.
- The native backend target limit is raised from 64 to 160 UIDs, while oversized KPM control payloads stay fail-safe.
- The app now blocks Native selections beyond 159 resolved app UIDs, reserves one backend slot for VPN Hide, and prevents secondary-profile copies from changing the shared configuration.
- Diagnostics no longer treats detection vectors that no active backend can cover on this device as errors: the Dashboard stays clean when the active module hides everything it can, and such vectors are shown neutrally ("not covered") in the detailed breakdown instead of as red leaks.

### Fixed
- VPN interfaces named for Tailscale, ZeroTier, and Hurricane Electric IPv6 tunnels are now hidden consistently across every backend.
- Stale SIOCGIFCONF buffer entries are cleared, so apps cannot recover hidden VPN interface names past the returned length.
- Apps in a user profile the system leaves nameless (cloned apps, private space) are labelled by profile type in the Hiding list instead of a bare user ID — rows like "Beeline (10)" read as leftovers from an already uninstalled app.
- The Dashboard no longer says the VPN is off when a diagnostics run actually failed (root dropped); it shows a distinct retry prompt.
- Apps from work, private, clone, and secondary profiles no longer go missing from the Hiding list on ROMs with incomplete multi-user package enumeration.
- KPM rejects unvalidated kernel families before loading and reports the exact incompatibility in the app, and uses raw user-copy routines only where the kernel or hardware PAN makes them safe.
- KPM telemetry now reports failed userspace copies instead of returning a false successful byte count.
- Concurrent KPM configuration updates now use a race-free snapshot protocol.
- Malformed custom port policies no longer expand into all-port blocking.
- The app now detects when vpnhide.kpm was installed by itself instead of the complete KPM module ZIP and explains how to repair the installation.
- The Dashboard and Diagnostics screens now agree on why a check is blocked, and a pending self-restart is shown ahead of a VPN-off prompt.
- The Hiding screen now warns after Save when the native backend's resolved UID limit leaves some selected app profiles without Native protection.
- A target list that exceeds KernelPatch's 1024-byte control buffer is now rejected with a clear capacity error — by the activator before it is sent, and by the kernel and KPM backends on receipt — instead of being silently truncated, which could leave some selected apps unprotected.
- Native backend statistics no longer stop at a fixed KPM or kernel-module output buffer; a readout that does hit the limit ends on a complete record and is marked partial instead of being counted as whole.
- VPN Hide no longer targets packages that share a platform identity such as android.uid.system. Selecting one used to write a rule against every system component running under that same UID, which could break connectivity; vendor-preinstalled apps have ordinary app UIDs and are unaffected.
- The Dashboard detects missing or non-executable module activators directly, and Save reports a corrupted module bundle instead of silently skipping it.
- Zygisk native mode now works in 32-bit app processes: the module ships an armeabi-v7a build alongside arm64-v8a (previously NeoZygisk failed to load it into processes forked from zygote32, e.g. on Samsung devices).
- Diagnostics no longer reports an unobservable check as a pass or a leak: a push-callback that never arrives and a network-interface enumeration that throws are now shown as "not measured" instead of a misleading green or red verdict.
- KPM backend now hides ioctl(SIOCGIFINDEX) interface-index probes (used by if_nametoindex), closing a VPN-presence leak that the .ko and Zygisk backends already blocked.
- The app no longer fails to load its state when a config file lacks a trailing newline.
- Zygisk no longer leaks hidden VPN interfaces through scatter/gather netlink reads.

### Removed
- The obsolete decimal KPM load-argument format is gone; load-time configuration now uses the control v2 snapshot.
- The retired pre-v1.0 per-backend storage migration is gone; the canonical JSON config is now the only configuration source.

## v1.1.1

### Fixed
- Fixed incomplete VPN hiding on some devices and firmwares (notably many MediaTek-based and custom OEM ROMs), where several app-side checks could still detect the tunnel through legacy connectivity APIs.

## v1.1.0

### Added
- Detailed diagnostics can export a separate opt-in ZIP with active kernel boot images and partition metadata.
- The dashboard now diagnoses a KPM install stuck without loading (corrupted activator or a generic activator failure), showing which one and how to fix it — previously it silently sat inactive with no explanation.
- The dashboard now shows the installed kernel module's GKI variant on its card, and update warnings for an outdated kmod name the exact recommended zip to grab — so updating over an old kmod install no longer means guessing which variant you originally flashed.
- Diagnostics now checks that VPN Hide itself is routed through your VPN; if it has been split-tunnelled out, it asks you to add it to the tunnel instead of running meaningless checks.
- Diagnostics now includes an RTM_GETRULE check that detects a VPN leaking through per-app policy routing rules.
- Settings now has a full Hidden apps page that shows automatic app-hiding matches, manual hidden apps, and per-app auto-hide exclusions.
- The dashboard now offers one-tap module downloads: a button to grab the exact recommended zip from the latest release (plus a link to all releases), and a download button on outdated-version and wrong-variant module warnings.

### Changed
- Debug bundles now include a best-effort boot logcat excerpt for LSPosed and Vector attach diagnostics.
- Debug exports now include richer device, module and backend diagnostics, including decoded hook status/counter reports for KPM, Zygisk, LSPosed and Ports live state.
- Default debug APK is now R8-shrunk; use rawDebug for the old unminified debug build.
- Full system logcat recording now saves a diagnostic ZIP with device, backend, kernel, config, dmesg, and debug-capture context.
- Diagnostics now probes the legacy getNetworkInfo(TYPE_VPN) API and drops the uninformative system-proxy check.
- Java-level diagnostics now attribute who hid the VPN (hidden by the backend, or a leak) like the native checks, instead of a bare clean/fail.
- The detailed diagnostics screen now shows who hid the VPN on each check — the backend, SELinux, or nothing to hide — instead of a bare pass/fail.
- Compact single-row app bar (logo, title and actions on one line) that scales the brand down on very narrow screens; Apps-list filters moved out of the top-bar menu into chips above the list.
- On large screens the Dashboard expands its header into a larger brand and animates back to the compact bar when you switch tabs.
- The detailed diagnostics screen now shows what root saw on each native check next to the app's own read, and marks 'nothing to hide' distinctly from a backend win — so a SELinux-blocked check that has nothing to leak explains itself instead of looking protected.

### Fixed
- Auto-hidden VPN apps are now re-detected on startup and when you tap Refresh, so a VPN app installed after the app list was cached gets hidden from detector apps without re-saving the hiding list.
- Dashboard now explains inactive Ports rules using the portshide load_status.
- Diagnostic captures now restore temporary debug logging reliably after errors, cancellation, or overlapping captures.
- Portshide diagnostics now record the latest apply status and command output.
- Dashboard native/Java level tiles no longer mislabel an unloaded backend as "Partial" or a mostly-working layer as "Not working": each layer now reads OK / Partial / Not active, judged by what actually hid the VPN.
- KPM activation now works on FolkPatch/APatch setups that expose the trusted su KernelPatch control token, without requiring users to save a SuperKey when the runtime already grants it.
- Protection search now closes on back, and unsaved app role toggles no longer move rows between groups before Save.

## v1.0.0

### Added
- New KPM (KernelPatch Module) native backend (beta): hides VPN interfaces — and the VPN server's public host-route — on non-GKI and module-signing-locked kernels (4.14 through 6.12). It flashes as a Magisk/KernelSU-Next/APatch module that loads at boot and applies your targets automatically (under APatch it waits for the superkey from the app).
- New Statistics tab: a per-app view of which apps probe for the VPN and how, with app icons and names like the Hiding tab, and a tap-through per-hook breakdown that explains each detection method. Capture sessions surface probes live while you use a target app, and stopping a session keeps its results on screen for review.
- Per-app hook selection: choose which Java and native hooks apply to each protected app, grouped by detection method with an explanation of what each method is and how an app uses it to detect a VPN. Full role labels (Java / Native / Apps / Ports) are shown by default.
- The dashboard now explains more failure modes instead of surfacing raw errors: a kernel that rejects the module because it enforces module signatures (EKEYREJECTED — it recommends KernelSU Next), a KPM installed but waiting for the APatch superkey, and a hiding layer that is active while some of its runtime checks still fail (a Details button opens the full diagnostics). A fresh install with no targets now reads as guidance rather than an error. It also detects private AOSP fields broken by a new Android release at install time, listing the affected fields and Android SDK so you can file a bug.
- App hiding gains an advanced manual picker for choosing which apps to hide, automatic hiding of installed VPN apps via configurable heuristics, cleanup of configured apps that are no longer available, and plaintext export of the package list.
- Settings can now export and import the full canonical JSON configuration, so you can back up or move your entire setup as a single file.
- Add a Settings reset button that removes the config, state and target files VPN Hide writes to the device, so nothing is left orphaned after you uninstall the app and its modules.
- Optional daily background update check that notifies you when a new version is available.
- Configure which localhost port ranges the Ports hiding role blocks.
- New Community & feedback screen gathers the author's contacts (GitHub, Telegram, 4PDA) in one place.
- Experimental local HTTP bridge with a host MCP server for adb-driven app-state and hiding management. Off by default; enable it under Settings → Developer → Agent control.
- Native libraries now align their LOAD segments on 16 KiB, so they load cleanly on 16 KiB-page devices such as Pixel 8 Pro on Android 16 and future hardware, without the "ELF LOAD not aligned" warning at app start.

### Changed
- Redesigned the app with a Material 3 Expressive UI: grouped cards across Dashboard, Hiding, Diagnostics, and Settings, an at-a-glance status summary, a refreshed top bar and app mark, a proper themed monochrome chameleon icon, haptic and animated navigation, surface colors that avoid overly black Material You backgrounds, and live theme controls for card shadows and animations. The 'Root access required' screen matches the new look and gains a 'Check again' button to re-probe root without reopening the app. The dashboard is decluttered, with low-priority notes shown as neutral info rather than warnings.
- Reframed the whole app around hiding the VPN: the Dashboard status reads “VPN hidden / VPN visible”, the Protection tab is renamed Hiding, and the Russian and English wording was polished throughout.
- The three Hiding tabs (Tun, App hiding, Ports) are now a single app list: each row carries J/N/A/P toggles (Java, Native, App-hiding, Ports) and one Save applies everything at once, so there's no more hunting an app across tabs or saving three times. Filters keep already-configured apps visible, list sorting is configurable (configured apps first by default), the in-screen help is better formatted, selected apps temporarily missing from the list are preserved on Save, and the list shows a retry card instead of spinning forever if it fails to load.
- Diagnostics now lives under Settings, alongside a new Developer section: a debug-logging preference (off by default to keep logcat quiet and save resources; when enabled it turns on verbose kmod dmesg output and LSPosed hook logs, which are also captured in debug exports) plus a toggle to mute version and changelog notices on dev builds. The dashboard now waits for the full protection-check set and shows an in-app loading state instead of holding the splash screen. Diagnostics also shows a distinct checks-failed retry state on a failed run instead of misreporting an active VPN as off.
- The Dashboard now models your setup as one Java backend (LSPosed) and one Native backend (kmod, KPM, or Zygisk) instead of a per-module list. It recognizes the new KPM backend and warns when more than one native backend is installed — an error for the kmod+KPM combination, which can freeze the kernel — and KPM reports a truthful conflict status when it stands down for a co-installed kernel module.
- On non-GKI kernels (4.14–5.4) the app now recommends the KPM backend (beta) instead of Zygisk.
- The Zygisk module now appears as VPN Hide (Zygisk) in your root manager.

### Fixed
- Closed several Java Connectivity detection vectors so more apps can no longer see the VPN: network-callback pushes (e.g. VTB), getNetworkInfo(TYPE_VPN) (e.g. Улыбка радуги), the legacy NetworkInfo API (getActiveNetworkInfo), and the VPN Network handles from getActiveNetwork/getAllNetworks are now sanitized for target apps. The system_server hooks scrub results through public APIs, so they also cover Android 17, which changed the private fields the old hooks read.
- Target apps can no longer detect a hidden VPN by enumerating interfaces or reading routes directly from the kernel. VPN routes — and the physical host-route hints that expose them — are stripped from RTM_GETROUTE netlink dumps (including the FORTIFY'd recvfrom/__recvfrom_chk path) and from /proc/net/ipv6_route, the SIOCGIFCONF buffer-size count trick (ifc_req == NULL) is closed, and tunnels renamed to the kernel-default `if<N>` pattern (issue #86) — along with utun/l2tp/gre and renamed *vpn* interfaces — are recognized and hidden consistently across the kmod, native, and Java backends. Hiding interfaces from RTM_GETLINK dumps also no longer hangs under Permissive SELinux.
- Apps you hide stay invisible to other observer apps while still seeing themselves, and Android 17's PackageManager list wrappers are preserved when hiding packages.
- A malformed port rule in the stored config no longer discards the whole configuration (which silently disabled every hook), and selected-app UIDs now resolve by literal package match so similarly named packages are never targeted by mistake. The activator also warns on stderr when native targets exceed the 64-target backend cap, instead of silently dropping the highest-UID apps from native protection.
- Fixed the app failing to start with "Startup preparation failed" — the root setup command was being flattened to one line, breaking its shell syntax. The failure screen now explains the actual cause (root unavailable, incomplete root data, or a config-write failure) and shows the underlying error detail instead of always telling you to check root permissions.
- Fixed an out-of-memory crash when running diagnostics with a VPN connected on some kernels: the native netlink interface/route checks now bound their read loop and time out instead of looping until the allocator aborts.

### Security
- Localhost port blocking now covers the entire 127.0.0.0/8 loopback range, so a proxy or VPN daemon bound to 0.0.0.0 can no longer be reached through a 127.x.x.x alias.
- LSPosed hides a protected package from getNameForUid / getNamesForUids uid-to-name resolution and scrubs the VPN's DNS servers and tunnel addresses from LinkProperties.
- Zygisk blocks VPN interface-index probes (if_nametoindex / ioctl(SIOCGIFINDEX)) and intercepts /proc/net/{dev,udp,udp6} and /proc/thread-self and task path forms that could reveal a hidden VPN, and no longer reads out of bounds on netlink replies that use MSG_TRUNC.

## v0.7.1

### Added
- Expanded Russian app detection: Anixart, ePN Cashback, TNT Premier, Swoo (Кошелёк), Макси, Ростелеком Личный кабинет, Проверка чеков ФНС

### Changed
- Reduced APK size by ~93% (47 MB → 3.37 MB) by enabling R8 minification, resource shrinking, and replacing the com.google.android.material dependency with AndroidX SplashScreen; the splash screen now holds through startup until the Dashboard is ready, removing transient loading spinners on cold launch
- Show profile names (Work, Second Space, ...) next to apps installed in other user profiles, instead of raw user IDs

### Fixed
- Kmod install recommendation no longer falsely pushes users to Zygisk on custom kernels whose `uname -r` lacks the GKI KMI tag (e.g. `android12`, `android13`). The heuristic now matches only the parsed KMI — not the phone's Android OS release, which is an unrelated label — and falls back on kernel series when the KMI is missing: 6.1 / 6.6 / 6.12 each ship a single KMI variant and are still unambiguous; 5.10 and 5.15 each have two candidates, both of which the app now surfaces (primary + alternative), with a dedicated banner when the installed variant fails to load so the user knows to try the other. An active kmod — `/proc/vpnhide_targets` present — also overrides any remaining heuristic-driven warning.
- Polish multi-profile app list: the Show system filter now classifies apps installed only in a secondary profile correctly, and the user-ID suffix no longer appears on every row for users without a secondary profile

## v0.7.0

### Added
- Dashboard now splits issues into Errors (red, block protection) and Warnings (amber, setup is suboptimal but working). Four new warnings: kernel supports kmod but only Zygisk is installed; kmod and Zygisk both active simultaneously (means you have to remember per-app Z-off for banking / payment apps that detect Zygisk); debug logging left on; SELinux in Permissive mode (exposes six detection vectors that VPN Hide relies on the kernel to block).
- Debug logging toggle in Diagnostics: off by default — VPN Hide, LSPosed hooks (VpnHide-NC/NI/LP and the package-visibility filter), and zygisk keep logcat near-silent. Start recording and Collect debug log automatically enable verbose logging for the duration of the capture and restore it afterwards, so the toggle is only needed if you want logs emitted continuously outside a capture. Errors always pass through so hook-install failures remain visible.
- Diagnose wrong-variant kmod installs: Dashboard surfaces when the installed kernel module is built for a different GKI variant than your kernel (e.g. android13-5.10 installed on android14-6.1), when your kernel is missing kretprobes, or has no compatible kmod variant at all — and points to the right zip. The same boot-time insmod status is bundled into the Collect debug log zip on the Diagnostics screen for bug reports.
- Expand Russian-apps filter: Pribyvalka 63, RosDomofon, Drivee, Setka, Twinby, GoodLine (4pda user feedback).
- Expand Russian-apps filter: Youla, Delivery Club, SDEK, Russian Post, Dom.ru, ConsultantPlus, etc. — local retailers, pharmacies, food chains and services.
- Multi-profile support: VPN Hide now targets every instance of a selected app across user profiles (work profile, MIUI Second Space, Private Space, secondary users). Previously hooks only matched the app in your primary profile — a work-profile Ozon would still see the VPN. Package-to-UID resolution at boot + at Save time now uses 'pm list packages -U --user all' and writes every UID to targets, so all profiles are covered automatically without any UI changes. Fixes the work-profile request in issue #15.

### Changed
- Changelog entries now live as per-PR Markdown fragments under changelog.d/ instead of a shared JSON section, and CHANGELOG.md is regenerated only at release time, so concurrent PRs no longer conflict on the changelog.
- Dashboard no longer re-runs all checks (module detection, kprobes, SELinux, target counts, GitHub update check, etc.) on every tab switch. State is loaded once at startup and cached in RAM; the Refresh button in the top bar forces a reload. Update check is cached for 6 hours and re-runs when the app comes back to the foreground after sitting in the background longer than that — no background jobs, no notifications, just reactive to app lifecycle.
- Help text on Protection screens (Tun / Apps / Ports) moved from a hard-to-discover ? icon in the top bar to always-visible collapsible cards at the top of each list. Users who read and understood the hints can collapse them — the state is remembered across app restarts.
- Installed-app list is now loaded once at app start and cached in RAM, so switching between Protection tabs is instant instead of waiting for the icon+label load every time. Added a refresh button in the top bar to force a reload (also re-reads the per-screen target/observer files).
- Diagnostics and Protection tabs now open instantly after the first load. Previously each tab switch re-ran all root-shell probes (module detection, target/observer files, pm lookups, 500ms of check functions), now those results live in process-lifetime caches. Diagnostics specifically: checks run once when VPN is first detected active, then results are fixed for the process — hooks don't change mid-session, so re-running is pointless. When VPN is off, both Dashboard and Diagnostics show a shared "turn on VPN, then Retry" banner. Log-capture tools (debug logging toggle, logcat recorder, debug-zip export) are now always visible on Diagnostics, regardless of VPN state.
- Diagnostics tab opens instantly even on the first time you tap it. The cache that stores the protection-check results now starts running in the background as soon as the app launches, instead of waiting for you to navigate to the tab — by the time you switch from Dashboard to Diagnostics, the results are usually already there.
- Tun help accordion now spells out which layers need a target-app restart after Save: L (LSPosed) and K (kmod) apply immediately, Z (Zygisk) hooks are per-process so any just-toggled target with Z needs a force-stop + reopen to pick up the change.

### Fixed
- App Hiding: marking the same app as both H (Hidden) and O (Observer) caused it to crash on startup — the app would query its own PackageInfo, our system_server hook matched it as an observer and stripped its own package from the result, and the framework bailed. Roles are now mutually exclusive: toggling one clears the other, and existing H+O configs are migrated to O-only on first launch.
- Dashboard now shows a consistent version string for all modules. Kernel-module, Zygisk and Ports module cards used to display the Magisk-style 'vX.Y.Z' from their module.prop, while the LSPosed hook module card showed the Android-style 'X.Y.Z' from the APK's versionName — on the same screen, for the same version number. The 'v' prefix is now stripped at parse time so every card reads 'X.Y.Z' (or 'X.Y.Z-N-gSHA' for dev builds).
- Dev builds of the app no longer trigger a false 'module version mismatch' warning on the Dashboard. The check now strips the git-describe dev suffix (e.g. 0.6.2-14-g1f2205e vs module 0.6.2) before comparing.
- Dev builds of the app now correctly receive "new version available" notifications. The comparison used to bail on the git-describe suffix (0.6.2-14-gSHA) and silently treat it as "no update", so testers running dev APKs never saw release prompts.
- Protection toolbar: the filter icon indicator for "filter is applied" no longer blends into the topbar background on Material You palettes. Active state is now a FilledIconButton (primary / onPrimary pair, guaranteed contrast) instead of a plain icon tinted with primary on top of primaryContainer.

## v0.6.2

### Added
- Full system logcat recording on the Diagnostics screen — Start/Stop an unfiltered capture of all logcat buffers (main/system/crash/events/radio), then Save to a user-picked location via the Storage Access Framework or Share through the system sheet. Useful for submitting diagnostic logs straight from the app without running adb logcat by hand.

### Changed
- Module zips and APK now include git provenance in the version string. Official release builds from a tag stay at the clean version (e.g. 0.6.2); intermediate builds from main or a feature branch show up as 0.6.2-5-gabc1234 (or -dirty) in the Magisk/KSU manager and Android Settings, so debug-log submissions identify the exact commit.

### Fixed
- Target apps (Ozon, Home Assistant, Megafon, Chrome, etc.) no longer hang on infinite load when vpnhide-zygisk is installed. Our recv/recvmsg hooks used to run the netlink-dump filter on every socket regardless of type — on TCP/UDP/Unix the first bytes are arbitrary user data (TLS ciphertext, HTTP body) and occasionally matched RTM_NEWLINK/RTM_NEWADDR by chance, causing the filter to mangle the buffer in-place and corrupt the TLS stream. Gated both hooks on AF_NETLINK so non-netlink traffic passes through untouched.
- Target apps no longer fail Java-level hooks on cold boot. service.sh in the kmod and zygisk modules used to resolve package names to UIDs as soon as pm list packages started responding — but PackageManager responds early with only the system-package snapshot, so user-installed targets (including the vpnhide app itself) got resolved to empty. /data/system/vpnhide_uids.txt was written with at most one UID, the LSPosed hook cached that empty set on its first writeToParcel call, and the Java-level filtering silently no-opped until the next lucky boot. Now service.sh waits until the vpnhide package itself is visible in pm list packages -U, so the full user-package index is ready before resolving.
- Ozon and other apps with root-detection scanning /proc/self/fd no longer hang when vpnhide-zygisk is installed. The module's zygote-side on_load was leaking the module-dir fd, which every forked app process inherited — root-tamper scans detected a descriptor pointing inside /data/adb/modules/ and refused to continue. The fd is now explicitly closed before any app fork.

## v0.6.1

### Added
- Detect gb.sweetlifecl as a Russian-vendor package in the app picker filter, so it shows up under the Russian-only filter alongside other domestic apps.

### Fixed
- Magisk versions before v28 failed to install vpnhide-ports and vpnhide-kmod with an unpack error — restored the META-INF/com/google/android/{update-binary,updater-script} entries the older managers expect.

## v0.6.0

### Added
- App hiding mode in Protection — hide selected apps from selected observer apps at the PackageManager level. Observer apps can no longer list, resolve, or query hidden apps.
- Ports hiding mode plus new vpnhide-ports.zip module — block selected apps from reaching 127.0.0.1 / ::1 ports to hide locally running VPN/proxy daemons (Clash, sing-box, V2Ray, Happ, etc.).
- Ports module integration in the dashboard — shows install state, active rules, observer count, and version mismatch/update warnings.

### Changed
- The old Apps tab is now Protection, split into three modes: Tun, Apps, and Ports.
- Ports rules apply immediately on Save and are restored automatically on boot.
- vpnhide-ports.zip is now included in the release/update pipeline with Magisk/KernelSU update metadata.

### Fixed
- Fixed LinkProperties filtering so VPN routes are stripped more reliably from app-visible network snapshots.
- Fixed SIOCGIFCONF filtering on some Android 12/13 5.10 kernels where the previous hook could succeed but never fire.
- Fixed debug log collection so app logcat entries are captured reliably on devices where logcat via su misses them.

## v0.5.3

### Added
- Debug log export — open the Diagnostics tab and tap "Collect debug log" at the bottom. The app gathers dmesg, check results, device info, module status, kernel symbols, targets, interfaces, routing tables, and logcat into a zip. Save to disk or share directly.
- Kernel module debug logging toggle — all 6 kretprobe hooks now log detailed info (UID, target status, interface name, filter decisions) when debug mode is active. Enabled automatically during debug log collection.

## v0.5.2

### Fixed
- Fixed SIOCGIFCONF filtering on kernel 5.10 (tun0 was visible in interface enumeration)
- Fixed zygisk first-launch race: dashboard no longer shows false "inactive" status
- Added recv hook in zygisk for netlink filtering on Android 10
- Fixed hardcoded v0.1.0 in module installer messages

## v0.5.1

### Fixed
- Fixed false "LSPosed/Vector not installed" warning when LSPosed uses non-standard module path (e.g. zygisk_lsposed)
- Fixed false LSPosed config warnings when hooks are already active at runtime
- "No target apps configured" now checks all modules, not just LSPosed

## v0.5.0

### Added
- Built-in diagnostics merged into the main app (separate test app removed)
- New app icon and chameleon mascot branding
- Dashboard with module status, LSPosed config validation, version checks, and live protection verification
- Native module install recommendation — the app detects your kernel and tells you exactly which module to download
- Per-app protection layer toggles (L/K/Z) — control LSPosed, kernel module, and Zygisk independently for each app
- Magisk/KSU auto-update for kmod and zygisk modules
- App update check via GitHub Releases
- Changelog with version history

### Changed
- Replaced WebUI with native Compose UI for module management
- Zygisk API lowered from v5 to v2 for Magisk v27 compatibility
- Apps tab: search in top bar, filter menu, fast scroll with letter indicator, Russian apps filter

### Fixed
- Fixed potential system_server crash caused by race condition in writeToParcel hooks
- App no longer removes itself from target lists when saving

## v0.4.2

### Added
- ioctl SIOCGIFMTU detection and filtering
- LinkProperties routes filtering in LSPosed hooks

### Fixed
- Fixed VPN LinkProperties deserialization crash

## v0.4.1

### Added
- Russian translation
- Root access error screen with clear instructions

## v0.4.0

### Added
- VPN Hide app with target picker UI — select apps to hide VPN from
- Built-in diagnostics — 26 checks covering all detection vectors
- Auto-detect VPN and auto-add self to target list

### Changed
- Replaced WebUI with native Compose UI

### Fixed
- Zygisk targets reading on Magisk (SELinux blocking /data/adb/)

## v0.3.1

### Changed
- CI renames APK artifacts to vpnhide-lsposed.apk and vpnhide-test.apk for clearer release downloads.

## v0.3.0

### Added
- Monorepo restructure — vpnhide-zygisk, vpnhide-lsposed, and vpnhide-kmod now live in a single repository with a unified release pipeline.
- Diagnostic test app (Compose) that exercises 23+ VPN-detection paths so you can verify the hooks are actually working on your device.
- Kernel module gains `dev_ifconf`, `inet_fill_ifaddr`, and `inet6_fill_ifaddr` kretprobes for complete native-level VPN interface hiding.
- Zygisk module now writes resolved UIDs to a shared file so the LSPosed system_server hooks can load them without reboot.

### Changed
- LSPosed module stripped down to system_server hooks only; native detection paths moved to zygisk and kmod where they belong.
- Native diagnostic checks rewritten from C++ to Rust to match the zygisk module's stack.
- Unified entire project under the MIT license.
- CI consolidated into a single workflow: parallel builds across all 7 GKI generations (Android 12/13/14/15/16 × 5.10/5.15/6.1/6.6/6.12) using the DDK container images.

### Fixed
- Removed leftover `/proc/net/*` redirect hooks from the LSPosed module — those paths are the zygisk/kmod job and were causing duplicate filtering.

## v0.2.0

### Added
- system_server-side hooks so Java API VPN hiding also works for apps using the MIR HCE SDK (which previously bypassed in-process hooks).
- Live UID reload via `FileObserver` (inotify) — adding or removing target apps no longer requires rebooting the device.

### Fixed
- `FileObserver` is now retained from GC and watches the parent directory (not the file) so edits-as-rename reliably fire the inotify event.
- UIDs are now read from `/data/system/` (readable by system_server under SELinux) and `writeToParcel` modifies the parcel in place instead of swapping the backing object.

## v0.1.0

### Added
- Initial release — LSPosed module that hides an active VPN from apps listed in the module's LSPosed scope.
- In-process Java hooks for `NetworkCapabilities`, `NetworkInfo`, `LinkProperties`, DNS servers, HTTP proxy, and VPN-related system properties.
- Hook on `NetworkCapabilities.toString()` strips VPN-identifying tokens from debug representations.
